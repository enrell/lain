//! `lain.social.activity@1`: the activity log, the friends feed and
//! notifications.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use lain_api::{SocialSettings, activity_type};
use lain_core::Error;
use lain_store::Bucket;

use crate::graph::{blocked_either, can_see_tx, friend_ids, get_settings, level_allows};
use crate::{
    Activity, Hidden, Notification, Rx, SUBJECT_ACTIVITY, Social, Tx, WorkRef, clamp_limit, get_json, hidden_work,
    input, invalid, need_user, normalize_work, out, pair, prefix, scan, store_err, work_key,
};

/// Ops of `lain.social.activity@1`.
pub mod activity_op {
    /// `RecordProgressInput` → `{recorded?}`
    pub const RECORD_PROGRESS: &str = "record-progress";
    /// `{user_id, before?, limit?, hidden_libraries?}` → `[Activity]`
    pub const FEED: &str = "feed";
    /// `{viewer_id, owner_id, before?, limit?, hidden_libraries?}` → `[Activity]`
    pub const USER_ACTIVITY: &str = "user-activity";
    /// `{user_id, limit?, hidden_libraries?}` → `{items, unread}`
    pub const NOTIFICATIONS: &str = "notifications";
    /// `{user_id, ids?, all?}` → the number changed
    pub const MARK_READ: &str = "mark-read";
}

/// One accepted progress write. `was_completed` is the stored state
/// before it, so completion is recorded only on the transition (D-084).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecordProgressInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default)]
    pub season: i64,
    #[serde(default)]
    pub episode: i64,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub was_completed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordProgressOutput {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recorded: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FeedInput {
    #[serde(default)]
    pub user_id: String,
    /// Unix seconds; rows strictly older. 0 reads from now.
    #[serde(default)]
    pub before: i64,
    #[serde(default)]
    pub limit: i64,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserActivityInput {
    #[serde(default)]
    pub viewer_id: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub before: i64,
    #[serde(default)]
    pub limit: i64,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NotificationsInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub limit: i64,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationsOutput {
    pub items: Vec<Notification>,
    pub unread: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MarkReadInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
}

/// Deletes the oldest keys under `p` beyond `keep` (keys under a prefix
/// are time-ordered ids).
pub(crate) fn trim_prefix(tx: &mut Tx<'_>, b: Bucket, p: &[u8], keep: usize) -> Result<(), Error> {
    let keys: Vec<Vec<u8>> = tx.pairs(b, p).map_err(store_err)?.into_iter().map(|(k, _)| k).collect();
    if keys.len() > keep {
        for k in &keys[..keys.len() - keep] {
            tx.delete(b, k)?;
        }
    }
    Ok(())
}

/// The per-row rules on top of activity visibility: rating rows also
/// need ratings visibility, progress rows vanish for accounts that hide
/// what they are watching now, and rows naming an item of a hidden
/// library are never shown (D-156), not even to their owner.
fn visible_row(rx: &impl Rx, viewer: &str, st: &SocialSettings, a: &Activity, hidden: &Hidden) -> Result<bool, Error> {
    if hidden_work(hidden, &a.work) {
        return Ok(false);
    }
    if viewer == a.user_id {
        return Ok(true);
    }
    match a.kind.as_str() {
        activity_type::PROGRESS => Ok(!st.hide_progress),
        activity_type::RATED => level_allows(rx, &st.ratings, viewer, &a.user_id),
        _ => Ok(true),
    }
}

/// Up to `limit` of `owner`'s rows older than `before`, newest first.
fn read_activity(
    rx: &impl Rx,
    viewer: &str,
    owner: &str,
    before: i64,
    limit: usize,
    hidden: &Hidden,
) -> Result<Vec<Activity>, Error> {
    let st = get_settings(rx, owner)?;
    let mut out = Vec::new();
    for (_, a) in scan::<Activity>(rx, Bucket::SocialActivity, &prefix(owner))?.into_iter().rev() {
        if before > 0 && a.at >= before {
            continue;
        }
        if visible_row(rx, viewer, &st, &a, hidden)? {
            out.push(a);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

fn sort_activity(a: &mut [Activity]) {
    a.sort_by(|x, y| y.at.cmp(&x.at).then_with(|| y.id.cmp(&x.id)));
}

impl Social {
    pub(crate) fn invoke_activity(&self, op: &str, v: Value) -> Result<Value, Error> {
        use activity_op::*;
        match op {
            RECORD_PROGRESS => {
                let i: RecordProgressInput = input(v, "RecordProgressInput")?;
                need_user(&[&i.user_id])?;
                out(self.record_progress(i)?)
            }
            FEED => {
                let i: FeedInput = input(v, "FeedInput")?;
                need_user(&[&i.user_id])?;
                out(self.feed(&i)?)
            }
            USER_ACTIVITY => {
                let i: UserActivityInput = input(v, "UserActivityInput")?;
                need_user(&[&i.viewer_id, &i.owner_id])?;
                out(self.user_activity(&i)?)
            }
            NOTIFICATIONS => {
                let i: NotificationsInput = input(v, "NotificationsInput")?;
                need_user(&[&i.user_id])?;
                out(self.notifications(&i.user_id, i.limit, &i.hidden_libraries.into_iter().collect())?)
            }
            MARK_READ => {
                let i: MarkReadInput = input(v, "MarkReadInput")?;
                need_user(&[&i.user_id])?;
                out(self.mark_read(&i)?)
            }
            _ => Err(invalid(format!("unknown op {op} of lain.social.activity@1"))),
        }
    }

    /// Appends one row and trims the account to its newest rows.
    pub(crate) fn append_activity_tx(&self, tx: &mut Tx<'_>, mut a: Activity) -> Result<(), Error> {
        a.id = self.new_id();
        if a.at == 0 {
            a.at = self.now();
        }
        tx.put(Bucket::SocialActivity, &pair(&a.user_id, &a.id), &a)?;
        trim_prefix(tx, Bucket::SocialActivity, &prefix(&a.user_id), crate::MAX_ACTIVITY_ROWS)
    }

    /// Turns one progress write into at most one row: `completed` on
    /// the transition, otherwise `progress` once per work per window, so
    /// a player reporting every few seconds floods nobody's feed.
    pub fn record_progress(&self, i: RecordProgressInput) -> Result<RecordProgressOutput, Error> {
        let w = normalize_work(i.work)?;
        let now = self.now();
        let last = pair(&i.user_id, &work_key(&w));
        let kind = if i.completed && !i.was_completed {
            activity_type::COMPLETED
        } else if !i.completed {
            let at: i64 = self.view(|r| get_json(r, Bucket::SocialActivityLast, &last))?.unwrap_or(0);
            if at == 0 || now - at >= crate::PROGRESS_WINDOW_SECS { activity_type::PROGRESS } else { "" }
        } else {
            ""
        };
        if kind.is_empty() {
            return Ok(RecordProgressOutput::default());
        }
        self.update(|tx| {
            tx.put(Bucket::SocialActivityLast, &last, &now)?;
            self.append_activity_tx(
                tx,
                Activity {
                    user_id: i.user_id.clone(),
                    kind: kind.into(),
                    work: w,
                    season: i.season,
                    episode: i.episode,
                    at: now,
                    ..Activity::default()
                },
            )
        })?;
        Ok(RecordProgressOutput { recorded: kind.into() })
    }

    /// Every friend's activity the viewer may see, newest first.
    pub fn feed(&self, i: &FeedInput) -> Result<Vec<Activity>, Error> {
        let limit = clamp_limit(i.limit);
        let hidden: Hidden = i.hidden_libraries.iter().cloned().collect();
        let mut out = self.view(|r| {
            let mut out = Vec::new();
            for f in friend_ids(r, &i.user_id)? {
                if !can_see_tx(r, &i.user_id, &f, SUBJECT_ACTIVITY)? {
                    continue;
                }
                out.extend(read_activity(r, &i.user_id, &f, i.before, limit, &hidden)?);
            }
            Ok(out)
        })?;
        sort_activity(&mut out);
        out.truncate(limit);
        Ok(out)
    }

    /// One account's rows as the viewer may see them; a hidden account
    /// reads as empty, never as an error that confirms it.
    pub fn user_activity(&self, i: &UserActivityInput) -> Result<Vec<Activity>, Error> {
        let hidden: Hidden = i.hidden_libraries.iter().cloned().collect();
        self.view(|r| {
            if !can_see_tx(r, &i.viewer_id, &i.owner_id, SUBJECT_ACTIVITY)? {
                return Ok(Vec::new());
            }
            read_activity(r, &i.viewer_id, &i.owner_id, i.before, clamp_limit(i.limit), &hidden)
        })
    }

    /// The newest notifications and the unread total. Rows from accounts
    /// now blocked either way, and rows naming an item of a hidden
    /// library (D-156), are skipped and not counted.
    pub fn notifications(&self, user_id: &str, limit: i64, hidden: &Hidden) -> Result<NotificationsOutput, Error> {
        let limit = clamp_limit(limit);
        self.view(|r| {
            let mut o = NotificationsOutput::default();
            for (_, n) in scan::<Notification>(r, Bucket::SocialNotifications, &prefix(user_id))?.into_iter().rev() {
                if !n.from_user_id.is_empty() && blocked_either(r, user_id, &n.from_user_id)? {
                    continue;
                }
                if n.work.as_ref().is_some_and(|w| hidden_work(hidden, w)) {
                    continue;
                }
                if !n.read {
                    o.unread += 1;
                }
                if o.items.len() < limit {
                    o.items.push(n);
                }
            }
            Ok(o)
        })
    }

    /// Flags the listed notifications (or all) read; returns how many
    /// changed.
    pub fn mark_read(&self, i: &MarkReadInput) -> Result<i64, Error> {
        self.update(|tx| {
            let rows: Vec<(String, Notification)> = scan(tx, Bucket::SocialNotifications, &prefix(&i.user_id))?;
            let mut changed = 0;
            for (id, mut n) in rows {
                if n.read || !(i.all || i.ids.contains(&n.id)) {
                    continue;
                }
                n.read = true;
                tx.put(Bucket::SocialNotifications, &pair(&i.user_id, &id), &n)?;
                changed += 1;
            }
            Ok(changed)
        })
    }
}
