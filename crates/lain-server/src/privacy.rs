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
//!   exists in `GET /api/libraries` (name, path, `private`, `locked`),
//!   can rename it, change owner and members, make it public or delete
//!   it, but see its items only as a member who unlocked it.
//! - Users who are neither admins nor members do not see the record at
//!   all; unlocking it answers `unknown library`.
//!
//! Unlocks live in memory only, keyed by session, and end when the
//! token expires, is rotated by a password change, or the server
//! restarts. Five failed unlock attempts in five minutes lock the user
//! out of further attempts for the rest of that window.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;

use lain_auth::Verified;

use crate::contracts::Library;

const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW_SECS: i64 = 300;

#[derive(Default)]
struct Session {
    user_id: String,
    expires_at: i64,
    libraries: BTreeSet<String>,
}

/// Per-session unlocks and the unlock attempt limiter.
#[derive(Default)]
pub struct Unlocks {
    sessions: Mutex<HashMap<String, Session>>,
    failures: Mutex<HashMap<String, VecDeque<i64>>>,
}

impl Unlocks {
    pub fn unlock(&self, v: &Verified, library_id: &str) {
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let now = lain_core::now_unix();
        s.retain(|_, x| x.expires_at > now);
        let e = s.entry(v.session.clone()).or_insert_with(|| Session {
            user_id: v.user_id.clone(),
            expires_at: v.expires_at,
            ..Session::default()
        });
        e.libraries.insert(library_id.to_string());
    }

    pub fn lock(&self, v: &Verified, library_id: &str) {
        if let Some(e) = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&v.session) {
            e.libraries.remove(library_id);
        }
    }

    pub fn is_unlocked(&self, user_id: &str, session: &str, library_id: &str) -> bool {
        let s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        s.get(session).is_some_and(|x| {
            x.user_id == user_id && x.expires_at > lain_core::now_unix() && x.libraries.contains(library_id)
        })
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

/// The record is listed to this user.
pub fn record_visible(lib: &Library, user_id: &str, admin: bool) -> bool {
    !lib.private || admin || lib.is_member(user_id)
}

/// The items are visible in this session.
pub fn items_visible(lib: &Library, unlocks: &Unlocks, user_id: &str, session: &str) -> bool {
    !lib.private || (lib.is_member(user_id) && unlocks.is_unlocked(user_id, session, &lib.id))
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
        assert!(!items_visible(&lib, &u, "u1", "s1"));
        u.unlock(&viewer("u1", "s1"), "lib-p");
        assert!(items_visible(&lib, &u, "u1", "s1"));
        assert!(!items_visible(&lib, &u, "u1", "s2"), "another session of the same user");
        assert!(!items_visible(&lib, &u, "u3", "s1"), "a session id is bound to its user");
        u.lock(&viewer("u1", "s1"), "lib-p");
        assert!(!items_visible(&lib, &u, "u1", "s1"));
        assert!(
            record_visible(&lib, "u2", false) && record_visible(&lib, "u9", true) && !record_visible(&lib, "u9", false)
        );
        let expired = Verified { expires_at: lain_core::now_unix() - 1, ..viewer("u2", "s9") };
        u.unlock(&expired, "lib-p");
        assert!(!items_visible(&lib, &u, "u2", "s9"));
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
