//! Private libraries (D-156): who may see what.
//!
//! - A library is **private** when its record says so. Its items never
//!   reach anyone who has not unlocked it **in this session** (this
//!   token): not the catalog and its totals, search, Continue, item
//!   routes (detail, episodes, thumbnails, sidecars, stream, pages),
//!   progress reads, or events. An item that is hidden answers exactly
//!   like one that does not exist.
//! - The **owner** (who made it private, or whom an admin names) and
//!   the **members** an admin adds may unlock it, with their own PIN or
//!   password. Nobody else can, admins included.
//! - **Admins** still manage the record: they see that the library
//!   exists in `GET /api/libraries` (`private`, `locked`), can rename it,
//!   change owner and members, make it public or delete it, but see its
//!   items only as a member who unlocked it. A **manager** (D-296) sees
//!   a private record the same way when their allow-list and audience
//!   ceiling permit it, and is not a member unless named as one.
//! - **A locked record gives nothing away** (D-293): its name and folder
//!   are blank in every answer until this session unlocks it, and
//!   changing or removing it takes the caller's PIN or password, unless
//!   this session has unlocked it.
//! - Users who are neither admins, managers, nor members do not see the
//!   record at all; unlocking it answers `unknown library`. A manager
//!   whose allow-list or audience ceiling excludes the library gets the
//!   same answer.
//!
//! Unlocks live in memory only, keyed by session, and end when the
//! token expires, is rotated by a password change, or the server
//! restarts.
//!
//! An unlock is a **lease on a live client** (D-294): it holds while
//! the session has an event stream open (`GET /api/events`, the
//! connection every client keeps for as long as it is on screen) and
//! for a short grace after the last one closes (a page reload). The
//! server ends it by itself; no client has to say "lock", so a dropped
//! or intercepted request can only lock sooner, never keep a library
//! open. Closing the tab or the browser, or losing the network, locks.
//!
//! Five failed unlock attempts in five minutes lock the user
//! out of further attempts for the rest of that window.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lain_auth::Verified;

use crate::contracts::Library;

const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW_SECS: i64 = 300;

/// How long an unlock outlives the session's last event stream: a page
/// reload, or the moment between unlocking and the stream connecting.
pub const LEASE_GRACE: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Session {
    user_id: String,
    expires_at: i64,
    libraries: BTreeSet<String>,
    /// Open event streams of this session.
    streams: usize,
    /// Since when it has none; the lease ends `grace` after.
    idle_since: Option<Instant>,
}

impl Session {
    fn live(&self, grace: Duration) -> bool {
        self.streams > 0 || self.idle_since.is_some_and(|t| t.elapsed() < grace)
    }
}

/// Per-session unlocks and the unlock attempt limiter.
pub struct Unlocks {
    sessions: Mutex<HashMap<String, Session>>,
    failures: Mutex<HashMap<String, VecDeque<i64>>>,
    grace: Duration,
}

impl Default for Unlocks {
    fn default() -> Self {
        Unlocks::with_grace(LEASE_GRACE)
    }
}

impl Unlocks {
    pub fn with_grace(grace: Duration) -> Self {
        Unlocks { sessions: Mutex::default(), failures: Mutex::default(), grace }
    }

    fn entry<'a>(s: &'a mut HashMap<String, Session>, v: &Verified) -> &'a mut Session {
        let now = lain_core::now_unix();
        s.retain(|_, x| x.expires_at > now || x.streams > 0);
        s.entry(v.session.clone()).or_insert_with(|| Session {
            user_id: v.user_id.clone(),
            expires_at: v.expires_at,
            idle_since: Some(Instant::now()),
            ..Session::default()
        })
    }

    pub fn unlock(&self, v: &Verified, library_id: &str) {
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let grace = self.grace;
        let e = Self::entry(&mut s, v);
        if !e.live(grace) {
            // What an ended lease held is gone; this unlock starts anew.
            e.libraries.clear();
        }
        if e.streams == 0 {
            e.idle_since = Some(Instant::now());
        }
        e.libraries.insert(library_id.to_string());
    }

    /// An event stream of this session opened: its unlocks hold.
    pub fn stream_opened(&self, v: &Verified) {
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let grace = self.grace;
        let e = Self::entry(&mut s, v);
        if !e.live(grace) {
            e.libraries.clear();
        }
        e.streams += 1;
        e.idle_since = None;
    }

    /// One closed; after the last, the unlocks end `grace` later.
    pub fn stream_closed(&self, session: &str) {
        if let Some(e) = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get_mut(session) {
            e.streams = e.streams.saturating_sub(1);
            if e.streams == 0 {
                e.idle_since = Some(Instant::now());
            }
        }
    }

    /// The session signed out (D-298): nothing of it stays unlocked.
    pub fn forget(&self, session: &str) {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(session);
    }

    pub fn lock(&self, v: &Verified, library_id: &str) {
        if let Some(e) = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&v.session) {
            e.libraries.remove(library_id);
        }
    }

    pub fn is_unlocked(&self, user_id: &str, session: &str, library_id: &str) -> bool {
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let Some(x) = s.get_mut(session) else { return false };
        if !x.live(self.grace) {
            x.libraries.clear();
            return false;
        }
        x.user_id == user_id && x.expires_at > lain_core::now_unix() && x.libraries.contains(library_id)
    }

    /// Whether the user may try again.
    pub fn may_try(&self, user_id: &str) -> bool {
        let mut f = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        let now = lain_core::now_unix();
        let q = f.entry(user_id.to_string()).or_default();
        while q.front().is_some_and(|t| now - t > FAILURE_WINDOW_SECS) {
            q.pop_front();
        }
        q.len() < MAX_FAILURES
    }

    pub fn failed(&self, user_id: &str) {
        self.failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(user_id.to_string())
            .or_default()
            .push_back(lain_core::now_unix());
    }

    pub fn succeeded(&self, user_id: &str) {
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).remove(user_id);
    }
}

/// Who is asking for a library or item (D-295).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub user_id: String,
    /// Exempt from the allow-list and the audience ceiling. Admins only.
    pub admin: bool,
    /// Lists a private library without being a member (the record stays
    /// blank while locked). Admins only: a manager sees a private library
    /// only as a member, like any account (D-296).
    pub lists_private: bool,
    /// `None` = every library; `Some(ids)` = only those library ids (`Some([])` = none).
    pub libraries: Option<Vec<String>>,
    /// Highest audience level this account may see (`everyone`, `teen`, `mature`).
    pub max_audience: String,
}

impl Access {
    /// Unrestricted account access (admins, or fallback when user lookup is unavailable).
    pub fn unrestricted(user_id: impl Into<String>, admin: bool) -> Self {
        Self {
            user_id: user_id.into(),
            admin,
            lists_private: admin,
            libraries: None,
            max_audience: lain_api::AUDIENCE_MATURE.to_string(),
        }
    }

    pub fn from_user(user: &lain_auth::User) -> Self {
        let admin = user.role == lain_auth::ROLE_ADMIN;
        Self {
            user_id: user.id.clone(),
            admin,
            lists_private: admin,
            libraries: user.libraries.clone(),
            max_audience: user.effective_max_audience().to_string(),
        }
    }
}

/// Whether the account itself is allowed to see `lib` (library allow-list and
/// audience ceiling). Admins are never restricted by either control.
pub fn account_allows(lib: &Library, access: &Access) -> bool {
    if access.admin {
        return true;
    }
    if let Some(allowed) = &access.libraries
        && !allowed.iter().any(|id| id == &lib.id)
    {
        return false;
    }
    lain_api::audience_rank(&access.max_audience) >= lain_api::audience_rank(lib.effective_audience())
}

/// The record is listed to this user.
pub fn record_visible(lib: &Library, access: &Access) -> bool {
    account_allows(lib, access) && (!lib.private || access.lists_private || lib.is_member(&access.user_id))
}

/// The items are visible in this session.
pub fn items_visible(lib: &Library, unlocks: &Unlocks, access: &Access, session: &str) -> bool {
    account_allows(lib, access)
        && (!lib.private || (lib.is_member(&access.user_id) && unlocks.is_unlocked(&access.user_id, session, &lib.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(user: &str, session: &str) -> Verified {
        Verified {
            user_id: user.into(),
            role: "user".into(),
            session: session.into(),
            expires_at: lain_core::now_unix() + 60,
        }
    }

    /// A stored level that is not a level fails closed on both sides: the
    /// library reads as `mature`, the account as `everyone` (D-295).
    #[test]
    fn an_unknown_audience_level_fails_closed() {
        let lib = |audience: &str| Library { id: "l".into(), audience: audience.into(), ..Default::default() };
        let account = |max: &str| Access {
            user_id: "u".into(),
            admin: false,
            lists_private: false,
            libraries: None,
            max_audience: lain_auth::User { max_audience: max.into(), ..Default::default() }
                .effective_max_audience()
                .to_string(),
        };
        assert!(account_allows(&lib(""), &account("")));
        assert!(account_allows(&lib("mature"), &account("")));
        assert!(!account_allows(&lib("x-rated"), &account("teen")), "a garbled library is mature");
        assert!(account_allows(&lib("x-rated"), &account("mature")));
        assert!(!account_allows(&lib("teen"), &account("grown-up")), "a garbled account is everyone");
        assert!(account_allows(&lib("everyone"), &account("grown-up")));
    }

    /// The lease: an unlock holds while the session has a stream open
    /// and for the grace after the last one closes; then it is gone for
    /// good, even if a stream opens again. Another session of the same
    /// user never shares it.
    #[test]
    fn an_unlock_ends_with_the_sessions_last_stream() {
        let grace = Duration::from_millis(120);
        let wait = || std::thread::sleep(grace + Duration::from_millis(60));
        let u = Unlocks::with_grace(grace);
        let (tab, tv) = (viewer("ana", "s-laptop"), viewer("ana", "s-tv"));
        // No stream at all: the unlock lasts the grace only.
        u.unlock(&tab, "lib");
        assert!(u.is_unlocked("ana", "s-laptop", "lib"));
        wait();
        assert!(!u.is_unlocked("ana", "s-laptop", "lib"));
        // Two streams (two tabs): it holds until the last one closes.
        u.stream_opened(&tab);
        u.stream_opened(&tab);
        u.stream_opened(&tv);
        u.unlock(&tab, "lib");
        wait();
        assert!(u.is_unlocked("ana", "s-laptop", "lib"));
        assert!(!u.is_unlocked("ana", "s-tv", "lib"), "the same user's other login stays locked");
        u.stream_closed("s-laptop");
        wait();
        assert!(u.is_unlocked("ana", "s-laptop", "lib"), "one tab is still open");
        u.stream_closed("s-laptop");
        assert!(u.is_unlocked("ana", "s-laptop", "lib"), "a reload fits in the grace");
        u.stream_opened(&tab);
        wait();
        assert!(u.is_unlocked("ana", "s-laptop", "lib"));
        u.stream_closed("s-laptop");
        wait();
        assert!(!u.is_unlocked("ana", "s-laptop", "lib"));
        // Coming back later does not bring the unlock back.
        u.stream_opened(&tab);
        assert!(!u.is_unlocked("ana", "s-laptop", "lib"));
        u.stream_closed("s-nope");
    }

    #[test]
    fn unlocks_are_per_session_and_member_only() {
        let lib = Library {
            id: "lib-p".into(),
            private: true,
            owner: "u1".into(),
            members: vec!["u2".into()],
            ..Library::default()
        };
        let u = Unlocks::default();
        let u1 = Access::unrestricted("u1", false);
        let u2 = Access::unrestricted("u2", false);
        let u3 = Access::unrestricted("u3", false);
        let u9_admin = Access::unrestricted("u9", true);
        let u9_user = Access::unrestricted("u9", false);
        assert!(!items_visible(&lib, &u, &u1, "s1"));
        u.unlock(&viewer("u1", "s1"), "lib-p");
        assert!(items_visible(&lib, &u, &u1, "s1"));
        assert!(!items_visible(&lib, &u, &u1, "s2"), "another session of the same user");
        assert!(!items_visible(&lib, &u, &u3, "s1"), "a session id is bound to its user");
        u.lock(&viewer("u1", "s1"), "lib-p");
        assert!(!items_visible(&lib, &u, &u1, "s1"));
        assert!(record_visible(&lib, &u2) && record_visible(&lib, &u9_admin) && !record_visible(&lib, &u9_user));
        let expired = Verified { expires_at: lain_core::now_unix() - 1, ..viewer("u2", "s9") };
        u.unlock(&expired, "lib-p");
        assert!(!items_visible(&lib, &u, &u2, "s9"));
    }

    #[test]
    fn per_account_library_allow_list_and_audience_rules() {
        let mut everyone_lib = Library { id: "lib-all".into(), ..Library::default() };
        let teen_lib = Library { id: "lib-teen".into(), audience: "teen".into(), ..Library::default() };
        let mature_lib = Library { id: "lib-mature".into(), audience: "mature".into(), ..Library::default() };
        let u = Unlocks::default();

        // Kid account: max_audience = everyone, allow-list unset.
        let kid = Access {
            user_id: "kid".into(),
            admin: false,
            lists_private: false,
            libraries: None,
            max_audience: lain_api::AUDIENCE_EVERYONE.into(),
        };
        assert!(record_visible(&everyone_lib, &kid) && items_visible(&everyone_lib, &u, &kid, "s1"));
        assert!(!record_visible(&teen_lib, &kid) && !items_visible(&teen_lib, &u, &kid, "s1"));
        assert!(!record_visible(&mature_lib, &kid) && !items_visible(&mature_lib, &u, &kid, "s1"));

        // Teen account sees everyone + teen, not mature.
        let teen = Access {
            user_id: "teen".into(),
            admin: false,
            lists_private: false,
            libraries: None,
            max_audience: lain_api::AUDIENCE_TEEN.into(),
        };
        assert!(items_visible(&everyone_lib, &u, &teen, "s1"));
        assert!(items_visible(&teen_lib, &u, &teen, "s1"));
        assert!(!items_visible(&mature_lib, &u, &teen, "s1"));

        // Explicit allow-list restricts even audience-allowed libraries, and
        // empty list hides everything.
        let restricted = Access {
            user_id: "bro".into(),
            admin: false,
            lists_private: false,
            libraries: Some(vec!["lib-teen".into()]),
            max_audience: lain_api::AUDIENCE_MATURE.into(),
        };
        assert!(!items_visible(&everyone_lib, &u, &restricted, "s1"));
        assert!(items_visible(&teen_lib, &u, &restricted, "s1"));
        assert!(!items_visible(&mature_lib, &u, &restricted, "s1"));

        let empty = Access { libraries: Some(vec![]), ..restricted.clone() };
        assert!(!record_visible(&everyone_lib, &empty));
        assert!(!record_visible(&teen_lib, &empty));

        // Account restrictions override private membership + active unlock.
        everyone_lib.private = true;
        everyone_lib.owner = "bro".into();
        u.unlock(&viewer("bro", "s1"), "lib-all");
        assert!(!record_visible(&everyone_lib, &restricted));
        assert!(!items_visible(&everyone_lib, &u, &restricted, "s1"));

        // Admin ignores both library allow-list and max_audience restrictions.
        let admin = Access {
            user_id: "adm".into(),
            admin: true,
            lists_private: true,
            libraries: Some(vec![]),
            max_audience: lain_api::AUDIENCE_EVERYONE.into(),
        };
        assert!(record_visible(&teen_lib, &admin) && items_visible(&teen_lib, &u, &admin, "s1"));
        assert!(record_visible(&mature_lib, &admin) && items_visible(&mature_lib, &u, &admin, "s1"));

        // A manager lists a private library it is not a member of (blank
        // while locked, no items) but the allow-list and the ceiling still bind.
        let mut private_teen = teen_lib.clone();
        private_teen.private = true;
        private_teen.owner = "someone".into();
        let manager = Access {
            user_id: "mina".into(),
            admin: false,
            lists_private: true,
            libraries: Some(vec!["lib-teen".into()]),
            max_audience: lain_api::AUDIENCE_TEEN.into(),
        };
        assert!(record_visible(&private_teen, &manager));
        assert!(!items_visible(&private_teen, &u, &manager, "s1"));
        assert!(!record_visible(&everyone_lib, &manager), "outside the allow-list");
        assert!(!record_visible(&mature_lib, &manager), "above the audience ceiling");
        assert!(!record_visible(&private_teen, &restricted), "a user who is not a member still sees nothing");
    }

    #[test]
    fn attempts_are_limited() {
        let u = Unlocks::default();
        for _ in 0..MAX_FAILURES {
            assert!(u.may_try("u1"));
            u.failed("u1");
        }
        assert!(!u.may_try("u1"));
        assert!(u.may_try("u2"));
        u.succeeded("u1");
        assert!(u.may_try("u1"));
    }
}
