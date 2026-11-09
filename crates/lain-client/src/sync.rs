//! Progress sync: positions recorded while the server is unreachable
//! are queued in the client store and pushed when it answers again,
//! with explicit conflict rules for when another device wrote the same
//! item in the meantime.
//!
//! # Rules
//!
//! The server keeps one record per user and item and stamps it with
//! its own clock (`updated_at`); an online write simply replaces it
//! (**R0**, last writer wins on the server). Client clocks are never
//! compared with the server's: devices drift and WSL clocks jump, so a
//! timestamp race would pick winners at random.
//!
//! Queued records carry a **base**: the server record this client last
//! saw for the item (from a read, a write or a `progress-changed`
//! event). On flush the client reads the server record `S` and decides
//! for each queued record `Q`:
//!
//! - **R1 fast-forward.** `S` equals the base (nobody else wrote since):
//!   push `Q`, even when it moves backwards (a rewind on this device is
//!   intent). With no base, only an empty `S` counts as unchanged.
//! - Otherwise another device wrote meanwhile:
//!   - **R2 completion wins.** If exactly one side is completed, it
//!     wins: a completed `Q` is pushed; a completed `S` stays and `Q` is
//!     dropped. Finishing an episode is never undone by a partial
//!     position from elsewhere.
//!   - **R3 further position wins.** If neither is completed, the larger
//!     `position_sec` wins (on a tie the server's stays). Continuing is
//!     the common multi-device case, and a wrong pick costs a seek back
//!     rather than hunting for the place.
//!   - **R4 both completed.** The server's record stays.
//! - **R5 duration.** A pushed record with no `duration_sec` keeps the
//!   server's known duration.
//! - **R6 coalescing.** Several offline records of one item are one
//!   queue entry: the latest position wins, `completed` is sticky, and
//!   the base stays the one from before the first offline write.
//! - **R7 refusals.** A queued record the server refuses (a 4xx other
//!   than 401/429) is dropped and reported, so one bad record never
//!   blocks the queue. Offline, timeouts, 401, 429 and 5xx stop the
//!   flush and keep everything queued.
//!
//! Records are kept per server and per user: a queue is only ever
//! pushed with the token of the user who recorded it.

use std::sync::Arc;

use lain_api::{Progress, ProgressUpdate};
use serde::{Deserialize, Serialize};

use crate::api::Client;
use crate::db::{self, ClientDb, QUEUE, SEEN};
use crate::error::{ClientError, store_err};
use crate::events::ServerEvent;

/// One queued offline record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedProgress {
    pub item_id: String,
    pub update: ProgressUpdate,
    /// The server record seen before the first offline write (R1).
    pub base: Option<Progress>,
    /// The client's clock, for display only (never compared).
    pub recorded_at: i64,
    /// Queue order; it changes on every coalesced write.
    pub seq: u64,
}

/// Which rule decided a queued record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictRule {
    FastForward,
    CompletionWins,
    FurtherPositionWins,
    BothCompleted,
    Refused,
}

/// What happened to one queued record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The local record was written to the server.
    Pushed,
    /// The server's record stayed; the local one was dropped.
    KeptServer,
    /// The server refused the record; it was dropped (R7).
    Refused,
}

/// One flushed record.
#[derive(Debug, Clone, PartialEq)]
pub struct FlushItem {
    pub item_id: String,
    pub rule: ConflictRule,
    pub resolution: Resolution,
    /// The server's record after the decision (zero when refused).
    pub progress: Progress,
    pub message: String,
}

/// A flush pass.
#[derive(Debug, Clone, PartialEq)]
pub struct FlushReport {
    pub items: Vec<FlushItem>,
    /// Records still queued.
    pub remaining: u64,
    /// Why the pass stopped early (offline, 401, ...), if it did.
    pub stopped: Option<ClientError>,
}

/// What [`ProgressSync::record`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordOutcome {
    /// Written to the server.
    Synced { progress: Progress },
    /// Kept locally until the next flush.
    Queued { reason: String },
    /// A queued record of the item lost to the server's (R2–R4).
    KeptServer { progress: Progress },
}

/// The sync engine of one client.
#[derive(Clone)]
pub struct ProgressSync {
    client: Client,
    db: Arc<ClientDb>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn same(a: &Progress, b: &Progress) -> bool {
    a.updated_at == b.updated_at
        && a.position_sec == b.position_sec
        && a.duration_sec == b.duration_sec
        && a.completed == b.completed
}

/// R1–R5: whether `q` should be pushed (and as what) over `s`.
pub fn decide(q: &QueuedProgress, s: &Progress) -> (ConflictRule, Option<ProgressUpdate>) {
    let unchanged = match &q.base {
        Some(b) => same(b, s),
        None => s.updated_at == 0,
    };
    let mut push = q.update;
    if push.duration_sec == 0.0 && s.duration_sec > 0.0 {
        push.duration_sec = s.duration_sec;
    }
    if unchanged {
        return (ConflictRule::FastForward, Some(push));
    }
    match (q.update.completed, s.completed) {
        (true, false) => (ConflictRule::CompletionWins, Some(push)),
        (false, true) => (ConflictRule::CompletionWins, None),
        (true, true) => (ConflictRule::BothCompleted, None),
        (false, false) if q.update.position_sec > s.position_sec => (ConflictRule::FurtherPositionWins, Some(push)),
        (false, false) => (ConflictRule::FurtherPositionWins, None),
    }
}

/// R6: folds a newer local write into a queued one.
fn coalesce(
    old: Option<QueuedProgress>,
    item_id: &str,
    update: ProgressUpdate,
    base: Option<Progress>,
    seq: u64,
) -> QueuedProgress {
    match old {
        Some(o) => QueuedProgress {
            item_id: item_id.to_string(),
            update: ProgressUpdate { completed: update.completed || o.update.completed, ..update },
            base: o.base,
            recorded_at: now(),
            seq,
        },
        None => QueuedProgress { item_id: item_id.to_string(), update, base, recorded_at: now(), seq },
    }
}

enum Step {
    Done(FlushItem),
    Stop(ClientError),
}

impl ProgressSync {
    pub fn new(client: Client, db: Arc<ClientDb>) -> Self {
        ProgressSync { client, db }
    }

    fn user(&self) -> Result<String, ClientError> {
        self.client.session().map(|s| s.user_id).ok_or(ClientError::Unauthorized)
    }

    fn key(&self, user: &str, item_id: &str) -> String {
        db::key(&[&self.client.server(), user, item_id])
    }

    fn seen(&self, key: &str) -> Result<Option<Progress>, ClientError> {
        self.db.read(|tx| db::get(&tx.open_table(SEEN).map_err(store_err)?, key))
    }

    fn set_seen(&self, key: &str, p: &Progress) -> Result<(), ClientError> {
        self.db.write(|tx| db::put(&mut tx.open_table(SEEN).map_err(store_err)?, key, p))
    }

    fn queued_at(&self, key: &str) -> Result<Option<QueuedProgress>, ClientError> {
        self.db.read(|tx| db::get(&tx.open_table(QUEUE).map_err(store_err)?, key))
    }

    /// Queues `update` (coalesced, R6), noting local use of an offline
    /// copy. Returns the queued record.
    fn enqueue(&self, user: &str, item_id: &str, update: ProgressUpdate) -> Result<QueuedProgress, ClientError> {
        let key = self.key(user, item_id);
        let server = self.client.server();
        self.db.write(|tx| {
            let seq = db::next_seq(tx)?;
            let mut q = tx.open_table(QUEUE).map_err(store_err)?;
            let old: Option<QueuedProgress> = db::get(&q, &key)?;
            let base: Option<Progress> = db::get(&tx.open_table(SEEN).map_err(store_err)?, &key)?;
            let rec = coalesce(old, item_id, update, base, seq);
            db::put(&mut q, &key, &rec)?;
            drop(q);
            crate::offline::note_use(tx, &server, item_id, update.completed)?;
            Ok(rec)
        })
    }

    /// Records a position. Online and with nothing queued for the item,
    /// it is written straight to the server (R0). Unreachable or logged
    /// out, it is queued. With a record of the item already queued, the
    /// two are coalesced and resolved by the rules.
    pub async fn record(&self, item_id: &str, update: ProgressUpdate) -> Result<RecordOutcome, ClientError> {
        let finite = |v: f64| v.is_finite() && v >= 0.0;
        if !finite(update.position_sec) || !finite(update.duration_sec) {
            return Err(ClientError::Invalid { message: "position and duration must be finite and >= 0".into() });
        }
        let user = self.user()?;
        let key = self.key(&user, item_id);
        if self.queued_at(&key)?.is_some() {
            let q = self.enqueue(&user, item_id, update)?;
            return Ok(match self.resolve(&key, q).await {
                Step::Stop(e) => RecordOutcome::Queued { reason: e.to_string() },
                Step::Done(i) if i.resolution == Resolution::Pushed => RecordOutcome::Synced { progress: i.progress },
                Step::Done(i) if i.resolution == Resolution::KeptServer => {
                    RecordOutcome::KeptServer { progress: i.progress }
                }
                Step::Done(i) => return Err(ClientError::Api { status: 400, code: String::new(), message: i.message }),
            });
        }
        match self.client.put_progress(item_id, update).await {
            Ok(p) => {
                self.set_seen(&key, &p)?;
                let server = self.client.server();
                self.db.write(|tx| crate::offline::note_use(tx, &server, item_id, update.completed))?;
                Ok(RecordOutcome::Synced { progress: p })
            }
            Err(e)
                if e.is_offline()
                    || matches!(e, ClientError::Unauthorized | ClientError::Api { status: 500.., .. }) =>
            {
                self.enqueue(&user, item_id, update)?;
                Ok(RecordOutcome::Queued { reason: e.to_string() })
            }
            Err(e) => Err(e),
        }
    }

    /// Reads the server's record and remembers it as the base for later
    /// offline writes. Offline, it answers the queued local position
    /// over the last one seen.
    pub async fn fetch(&self, item_id: &str) -> Result<Progress, ClientError> {
        let user = self.user()?;
        let key = self.key(&user, item_id);
        match self.client.progress(item_id).await {
            Ok(p) => {
                self.set_seen(&key, &p)?;
                Ok(p)
            }
            Err(e) if e.is_offline() => {
                let seen = self.seen(&key)?.unwrap_or_default();
                Ok(match self.queued_at(&key)? {
                    Some(q) => Progress {
                        item_id: item_id.to_string(),
                        user_id: user,
                        position_sec: q.update.position_sec,
                        duration_sec: q.update.duration_sec,
                        completed: q.update.completed,
                        updated_at: seen.updated_at,
                    },
                    None => seen,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Feeds a server event: `progress-changed` updates the base, so a
    /// write from another device is not mistaken for "unchanged".
    pub fn observe(&self, event: &ServerEvent) -> Result<(), ClientError> {
        if let ServerEvent::ProgressChanged(e) = event {
            let user = self.user()?;
            self.set_seen(&self.key(&user, &e.item_id), &e.progress)?;
        }
        Ok(())
    }

    /// This user's queued records on this server, in queue order.
    pub fn queued(&self) -> Result<Vec<QueuedProgress>, ClientError> {
        let user = self.user()?;
        let p = db::prefix(&[&self.client.server(), &user]);
        let mut out: Vec<QueuedProgress> = self
            .db
            .read(|tx| db::scan(&tx.open_table(QUEUE).map_err(store_err)?, &p))?
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        out.sort_by_key(|q: &QueuedProgress| q.seq);
        Ok(out)
    }

    /// Pushes every queued record of this user, in queue order, until
    /// the server stops answering.
    pub async fn flush(&self) -> Result<FlushReport, ClientError> {
        let user = self.user()?;
        let mut items = Vec::new();
        let mut stopped = None;
        for q in self.queued()? {
            match self.resolve(&self.key(&user, &q.item_id), q).await {
                Step::Done(i) => items.push(i),
                Step::Stop(e) => {
                    stopped = Some(e);
                    break;
                }
            }
        }
        let remaining = self.queued()?.len() as u64;
        Ok(FlushReport { items, remaining, stopped })
    }

    /// Removes a queued record if no newer write replaced it meanwhile;
    /// a newer one gets `base` instead.
    fn settle(&self, key: &str, seq: u64, base: &Progress) -> Result<(), ClientError> {
        let server = self.client.server();
        self.db.write(|tx| {
            let mut q = tx.open_table(QUEUE).map_err(store_err)?;
            let cur: Option<QueuedProgress> = db::get(&q, key)?;
            match cur {
                Some(c) if c.seq == seq => {
                    q.remove(key).map_err(store_err)?;
                }
                Some(mut c) => {
                    c.base = Some(base.clone());
                    db::put(&mut q, key, &c)?;
                }
                None => {}
            }
            drop(q);
            db::put(&mut tx.open_table(SEEN).map_err(store_err)?, key, base)?;
            if base.completed {
                crate::offline::note_watched(tx, &server, &base.item_id)?;
            }
            Ok(())
        })
    }

    async fn resolve(&self, key: &str, q: QueuedProgress) -> Step {
        let s = match self.client.progress(&q.item_id).await {
            Ok(s) => s,
            Err(e) => return Step::Stop(e),
        };
        let (rule, push) = decide(&q, &s);
        let Some(update) = push else {
            return match self.settle(key, q.seq, &s) {
                Ok(()) => Step::Done(FlushItem {
                    item_id: q.item_id,
                    rule,
                    resolution: Resolution::KeptServer,
                    progress: s,
                    message: String::new(),
                }),
                Err(e) => Step::Stop(e),
            };
        };
        match self.client.put_progress(&q.item_id, update).await {
            Ok(p) => match self.settle(key, q.seq, &p) {
                Ok(()) => Step::Done(FlushItem {
                    item_id: q.item_id,
                    rule,
                    resolution: Resolution::Pushed,
                    progress: p,
                    message: String::new(),
                }),
                Err(e) => Step::Stop(e),
            },
            Err(e @ (ClientError::Forbidden { .. } | ClientError::NotFound { .. } | ClientError::Invalid { .. })) => {
                self.refuse(key, q, e)
            }
            Err(e @ ClientError::Api { status: 400..=499, .. }) => self.refuse(key, q, e),
            Err(e) => Step::Stop(e),
        }
    }

    fn refuse(&self, key: &str, q: QueuedProgress, e: ClientError) -> Step {
        tracing::warn!(item = %q.item_id, err = %e, "queued progress refused by the server; dropped");
        let res = self.db.write(|tx| {
            tx.open_table(QUEUE).map_err(store_err)?.remove(key).map_err(store_err)?;
            Ok(())
        });
        match res {
            Ok(()) => Step::Done(FlushItem {
                item_id: q.item_id,
                rule: ConflictRule::Refused,
                resolution: Resolution::Refused,
                progress: Progress::default(),
                message: e.to_string(),
            }),
            Err(se) => Step::Stop(se),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pos: f64, completed: bool, at: i64) -> Progress {
        Progress {
            item_id: "i".into(),
            user_id: "u".into(),
            position_sec: pos,
            duration_sec: 100.0,
            completed,
            updated_at: at,
        }
    }

    fn q(pos: f64, completed: bool, base: Option<Progress>) -> QueuedProgress {
        QueuedProgress {
            item_id: "i".into(),
            update: ProgressUpdate { position_sec: pos, duration_sec: 100.0, completed },
            base,
            recorded_at: 0,
            seq: 1,
        }
    }

    #[test]
    fn rules() {
        let base = p(50.0, false, 10);
        // R1: unchanged server, even a rewind is pushed.
        assert_eq!(decide(&q(5.0, false, Some(base.clone())), &base).0, ConflictRule::FastForward);
        assert!(decide(&q(5.0, false, Some(base.clone())), &base).1.is_some());
        assert_eq!(decide(&q(5.0, false, None), &Progress::default()).0, ConflictRule::FastForward);
        // No base and a server record: treated as a concurrent write.
        assert_eq!(decide(&q(5.0, false, None), &base), (ConflictRule::FurtherPositionWins, None));
        let moved = p(60.0, false, 11);
        // R2.
        assert_eq!(decide(&q(1.0, true, Some(base.clone())), &moved).0, ConflictRule::CompletionWins);
        assert!(decide(&q(1.0, true, Some(base.clone())), &moved).1.is_some());
        assert_eq!(
            decide(&q(99.0, false, Some(base.clone())), &p(1.0, true, 11)),
            (ConflictRule::CompletionWins, None)
        );
        // R3, ties keep the server.
        assert!(decide(&q(70.0, false, Some(base.clone())), &moved).1.is_some());
        assert!(decide(&q(60.0, false, Some(base.clone())), &moved).1.is_none());
        // R4.
        assert_eq!(decide(&q(1.0, true, Some(base.clone())), &p(2.0, true, 11)), (ConflictRule::BothCompleted, None));
        // R5.
        let mut nodur = q(5.0, false, Some(base.clone()));
        nodur.update.duration_sec = 0.0;
        assert_eq!(decide(&nodur, &base).1.unwrap().duration_sec, 100.0);
    }

    #[test]
    fn coalescing_keeps_base_and_completion() {
        let base = p(50.0, false, 10);
        let first = coalesce(
            None,
            "i",
            ProgressUpdate { position_sec: 99.0, duration_sec: 100.0, completed: true },
            Some(base.clone()),
            1,
        );
        let second = coalesce(
            Some(first),
            "i",
            ProgressUpdate { position_sec: 3.0, duration_sec: 100.0, completed: false },
            Some(p(1.0, false, 99)),
            2,
        );
        assert_eq!(second.update, ProgressUpdate { position_sec: 3.0, duration_sec: 100.0, completed: true });
        assert_eq!((second.base, second.seq), (Some(base), 2));
    }
}
