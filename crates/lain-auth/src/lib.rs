//! Accounts and session tokens, the Rust port of `internal/auth`.
//!
//! First-run setup creates the admin; admins create users. Passwords
//! are bcrypt (D-172; see `AGENTS.md` for why bcrypt over argon2):
//! hashes imported from the Go store keep verifying unchanged. Tokens
//! are HS256 JWTs signed with the store's secret and checked on every
//! request against live state: signature, expiry, account liveness and
//! password version. A password change bumps the version, so every
//! earlier token dies on its next use; disabling an account locks it
//! out within one request.

pub mod crypto;

use serde::{Deserialize, Serialize};

use lain_store::{Bucket, Store, StoreError};

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_USER: &str = "user";

/// bcrypt cost; Go's `bcrypt.DefaultCost`.
pub const BCRYPT_COST: u32 = 10;

/// Token lifetime.
pub const TOKEN_TTL_SECS: i64 = 24 * 3600;

/// The pre-made avatar set (D-086). The artwork lives in the clients.
pub const MASCOTS: [&str; 8] = ["wired", "moth", "static", "orbit", "glyph", "shell", "neon", "void"];

const MAX_DISPLAY_NAME: usize = 40;
const MAX_BIO: usize = 160;

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

/// Per-user playback limits (D-042). Unset means unrestricted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_video_transcode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_audio_transcode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remux: Option<bool>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub max_bitrate_kbps: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub max_streams: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle_mode: String,
}

/// The avatar shown for an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Avatar {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mascot: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub version: i64,
}

/// What other people see of an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bio: String,
    #[serde(default)]
    pub avatar: Avatar,
}

/// A local account, stored as the same JSON the Go server writes. The
/// hash is Go's `[]byte` encoding (standard base64); `null` when public.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub pass_hash: Option<String>,
    pub role: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub pwd_ver: u64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_language: String,
    #[serde(default)]
    pub playback: PlaybackPolicy,
    #[serde(default)]
    pub profile: Profile,
}

impl User {
    /// The record without its hash.
    pub fn public(mut self) -> User {
        self.pass_hash = None;
        self
    }

    fn hash_str(&self) -> Option<String> {
        let raw = crypto::b64_std_decode(self.pass_hash.as_deref()?)?;
        String::from_utf8(raw).ok()
    }
}

/// A profile change: only the fields present apply. `mascot: ""`
/// clears the avatar.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfilePatch {
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub mascot: Option<String>,
}

/// An authenticated identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub user_id: String,
    pub role: String,
}

/// An account failure. `Msg` texts are the Go server's, verbatim:
/// clients and the conformance suite read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    Msg(String),
    /// "unknown user" (gateways map it to 404 where Go does).
    UnknownUser,
    Store(StoreError),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Msg(m) => f.write_str(m),
            AuthError::UnknownUser => f.write_str("unknown user"),
            AuthError::Store(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AuthError {}

impl From<StoreError> for AuthError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Abort(m) if m == "unknown user" => AuthError::UnknownUser,
            StoreError::Abort(m) => AuthError::Msg(m),
            other => AuthError::Store(other),
        }
    }
}

fn msg(m: &str) -> AuthError {
    AuthError::Msg(m.to_string())
}

fn abort(m: &str) -> StoreError {
    StoreError::Abort(m.to_string())
}

/// Hashes a password the way Go's `bcrypt.GenerateFromPassword` does
/// (`$2a$`, cost 10, refusing more than 72 bytes).
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    if password.len() > 72 {
        return Err(msg("bcrypt: password length exceeds 72 bytes"));
    }
    let parts = bcrypt::hash_with_result(password, BCRYPT_COST).map_err(|e| AuthError::Msg(e.to_string()))?;
    Ok(parts.format_for_version(bcrypt::Version::TwoA))
}

/// Checks a password against a bcrypt hash (`$2a$`, `$2b$`, `$2y$`).
pub fn verify_password(password: &str, hash: &str) -> bool {
    if password.len() > 72 {
        return false;
    }
    bcrypt::verify(password, hash).unwrap_or(false)
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    role: String,
    pwdv: u64,
    iat: i64,
    exp: i64,
}

/// Trims, collapses whitespace (single-line) and rejects control
/// characters so a profile renders the same everywhere.
fn clean_text(s: &str, max: usize, field: &str, multiline: bool) -> Result<String, AuthError> {
    let s = s.trim();
    for c in s.chars() {
        if c == '\n' && multiline {
            continue;
        }
        if c.is_control() {
            return Err(AuthError::Msg(format!("{field} must not contain control characters")));
        }
    }
    let s = if multiline { s.to_string() } else { s.split_whitespace().collect::<Vec<_>>().join(" ") };
    if s.chars().count() > max {
        return Err(AuthError::Msg(format!("{field} is too long")));
    }
    Ok(s)
}

/// The account service over the store.
pub struct Auth<'s> {
    store: &'s Store,
    secret: Vec<u8>,
}

/// Loads the token secret, generating it once (`meta/secret`, raw
/// URL-safe base64 of 32 random bytes, as in Go).
pub fn load_or_create_secret(store: &Store) -> Result<Vec<u8>, AuthError> {
    let encoded = store.write(|tx| {
        if let Some(raw) = tx.get(Bucket::Meta, b"secret")? {
            return Ok(raw);
        }
        let mut fresh = [0u8; 32];
        crypto::random_bytes(&mut fresh).map_err(|e| StoreError::Abort(format!("secret: {e}")))?;
        let enc = crypto::b64_url(&fresh).into_bytes();
        tx.put(Bucket::Meta, b"secret", &enc)?;
        Ok(enc)
    })?;
    let text = String::from_utf8(encoded).map_err(|_| msg("auth: corrupt secret"))?;
    match crypto::b64_url_decode(&text) {
        Some(raw) if raw.len() >= 32 => Ok(raw),
        _ => Err(msg("auth: corrupt secret")),
    }
}

impl<'s> Auth<'s> {
    /// Binds the service to a store with an already loaded secret.
    pub fn new(store: &'s Store, secret: Vec<u8>) -> Self {
        Auth { store, secret }
    }

    fn get(&self, id: &str) -> Result<Option<User>, AuthError> {
        Ok(self.store.read(|tx| tx.get_json::<User>(Bucket::Users, id.as_bytes()))?)
    }

    fn put(&self, u: &User) -> Result<(), AuthError> {
        self.store.write(|tx| {
            tx.put_json(Bucket::Users, u.id.as_bytes(), u)?;
            tx.put(Bucket::UsersByName, u.username.as_bytes(), u.id.as_bytes())
        })?;
        Ok(())
    }

    /// Read-modify-write of one user in one transaction.
    fn update(&self, id: &str, f: impl FnOnce(&mut User) -> Result<(), AuthError>) -> Result<User, AuthError> {
        let mut user = None;
        let mut fail = None;
        self.store
            .write(|tx| {
                let Some(mut u) = tx.get_json::<User>(Bucket::Users, id.as_bytes())? else {
                    return Err(abort("unknown user"));
                };
                if let Err(e) = f(&mut u) {
                    fail = Some(e);
                    return Err(abort("rollback"));
                }
                tx.put_json(Bucket::Users, id.as_bytes(), &u)?;
                user = Some(u);
                Ok(())
            })
            .or_else(|e| match fail.take() {
                Some(f) => Err(f),
                None => Err(AuthError::from(e)),
            })?;
        Ok(user.expect("committed"))
    }

    /// Whether first-run setup already happened.
    pub fn has_users(&self) -> bool {
        self.store.read(|tx| tx.count(Bucket::Users)).map(|n| n > 0).unwrap_or(false)
    }

    fn new_user(id: String, username: &str, password: &str, role: &str) -> Result<User, AuthError> {
        if username.is_empty() || password.len() < 8 {
            return Err(msg("username required, password >= 8 chars"));
        }
        if role != ROLE_ADMIN && role != ROLE_USER {
            return Err(msg("unknown role"));
        }
        let hash = hash_password(password)?;
        Ok(User {
            id,
            username: username.to_string(),
            pass_hash: Some(crypto::b64_std(hash.as_bytes())),
            role: role.to_string(),
            pwd_ver: 1,
            created_at: lain_core::now_unix(),
            ..User::default()
        })
    }

    /// Creates the first admin; refuses once any user exists.
    pub fn setup(&self, username: &str, password: &str) -> Result<User, AuthError> {
        if self.has_users() {
            return Err(msg("setup already completed"));
        }
        let u = Self::new_user("user-admin".into(), username, password, ROLE_ADMIN)?;
        self.store
            .write(|tx| {
                // Re-check inside the transaction: two concurrent setups
                // must not both win.
                if tx.count(Bucket::Users)? > 0 {
                    return Err(abort("setup already completed"));
                }
                tx.put_json(Bucket::Users, u.id.as_bytes(), &u)?;
                tx.put(Bucket::UsersByName, u.username.as_bytes(), u.id.as_bytes())
            })
            .map_err(AuthError::from)?;
        Ok(u.public())
    }

    /// Adds a user. Usernames are unique; ids derive from them.
    pub fn create(&self, username: &str, password: &str, role: &str) -> Result<User, AuthError> {
        if self.store.read(|tx| tx.get(Bucket::UsersByName, username.as_bytes()))?.is_some() {
            return Err(msg("username taken"));
        }
        let u = Self::new_user(format!("user-{username}"), username, password, role)?;
        self.store
            .write(|tx| {
                if tx.get(Bucket::UsersByName, username.as_bytes())?.is_some() {
                    return Err(abort("username taken"));
                }
                tx.put_json(Bucket::Users, u.id.as_bytes(), &u)?;
                tx.put(Bucket::UsersByName, username.as_bytes(), u.id.as_bytes())
            })
            .map_err(AuthError::from)?;
        Ok(u.public())
    }

    /// All users without hashes, sorted by username. Undecodable
    /// records are skipped (as in Go).
    pub fn list(&self) -> Result<Vec<User>, AuthError> {
        let rows = self.store.read(|tx| tx.all(Bucket::Users))?;
        let mut out: Vec<User> =
            rows.iter().filter_map(|(_, v)| serde_json::from_slice::<User>(v).ok()).map(User::public).collect();
        out.sort_by(|a, b| a.username.cmp(&b.username));
        Ok(out)
    }

    /// The public record.
    pub fn get_public(&self, id: &str) -> Option<User> {
        self.get(id).ok().flatten().map(User::public)
    }

    pub fn set_disabled(&self, id: &str, disabled: bool) -> Result<(), AuthError> {
        self.update(id, |u| {
            u.disabled = disabled;
            Ok(())
        })
        .map(|_| ())
    }

    /// Changes the role; the token's role is re-read live on Verify.
    pub fn set_role(&self, id: &str, role: &str) -> Result<(), AuthError> {
        if role != ROLE_ADMIN && role != ROLE_USER {
            return Err(msg("unknown role"));
        }
        self.update(id, |u| {
            u.role = role.to_string();
            Ok(())
        })
        .map(|_| ())
    }

    pub fn set_playback(&self, id: &str, policy: PlaybackPolicy) -> Result<(), AuthError> {
        if policy.max_bitrate_kbps < 0 {
            return Err(msg("max_bitrate_kbps must not be negative"));
        }
        if policy.max_streams < 0 || policy.max_streams > 32 {
            return Err(msg("max_streams must be 0..32"));
        }
        match policy.subtitle_mode.as_str() {
            "" | "auto" | "extract" | "burn" | "off" => {}
            _ => return Err(msg("subtitle_mode must be auto, extract, burn or off")),
        }
        self.update(id, |u| {
            u.playback = policy;
            Ok(())
        })
        .map(|_| ())
    }

    /// Sets the ISO 639-2 viewing preference; empty clears it.
    pub fn set_preferred_language(&self, id: &str, language: &str) -> Result<(), AuthError> {
        let language = language.trim().to_lowercase();
        if !language.is_empty() && (language.len() != 3 || !language.bytes().all(|c| c.is_ascii_lowercase())) {
            return Err(msg("preferred_language must be a three-letter ISO 639-2 code"));
        }
        self.update(id, |u| {
            u.preferred_language = language;
            Ok(())
        })
        .map(|_| ())
    }

    pub fn playback_policy(&self, id: &str) -> PlaybackPolicy {
        self.get(id).ok().flatten().map(|u| u.playback).unwrap_or_default()
    }

    /// Verifies the old password, sets the new one and bumps the
    /// password version.
    pub fn change_password(&self, id: &str, old: &str, new: &str) -> Result<(), AuthError> {
        let u = self.get(id)?.ok_or(AuthError::UnknownUser)?;
        if !u.hash_str().is_some_and(|h| verify_password(old, &h)) {
            return Err(msg("invalid credentials"));
        }
        self.set_password(u, new)
    }

    /// Sets any user's password without the old one (admin reset).
    pub fn admin_reset(&self, id: &str, new: &str) -> Result<(), AuthError> {
        let u = self.get(id)?.ok_or(AuthError::UnknownUser)?;
        self.set_password(u, new)
    }

    fn set_password(&self, mut u: User, new: &str) -> Result<(), AuthError> {
        if new.len() < 8 {
            return Err(msg("password >= 8 chars"));
        }
        u.pass_hash = Some(crypto::b64_std(hash_password(new)?.as_bytes()));
        u.pwd_ver += 1;
        self.put(&u)
    }

    /// Applies a profile patch and returns the public record.
    pub fn update_profile(&self, id: &str, p: &ProfilePatch) -> Result<User, AuthError> {
        self.update(id, |u| {
            if let Some(v) = &p.display_name {
                u.profile.display_name = clean_text(v, MAX_DISPLAY_NAME, "display_name", false)?;
            }
            if let Some(v) = &p.bio {
                u.profile.bio = clean_text(v, MAX_BIO, "bio", true)?;
            }
            if let Some(m) = &p.mascot {
                let version = lain_core::now_unix_millis();
                if m.is_empty() {
                    u.profile.avatar = Avatar { version, ..Avatar::default() };
                } else if MASCOTS.contains(&m.as_str()) {
                    u.profile.avatar = Avatar { kind: "mascot".into(), mascot: m.clone(), version };
                } else {
                    return Err(msg("unknown mascot"));
                }
            }
            Ok(())
        })
        .map(User::public)
    }

    /// Verifies credentials and liveness, then mints a 24 h token.
    pub fn login(&self, username: &str, password: &str) -> Result<String, AuthError> {
        let invalid = || msg("invalid credentials");
        let u = self
            .store
            .read(|tx| {
                let Some(id) = tx.get(Bucket::UsersByName, username.as_bytes())? else { return Ok(None) };
                tx.get_json::<User>(Bucket::Users, &id)
            })
            .map_err(|_| invalid())?
            .ok_or_else(invalid)?;
        if u.disabled {
            return Err(msg("account disabled"));
        }
        if !u.hash_str().is_some_and(|h| verify_password(password, &h)) {
            return Err(invalid());
        }
        Ok(self.mint(&u, lain_core::now_unix()))
    }

    /// Signs a token for `u` issued at `now`.
    pub fn mint(&self, u: &User, now: i64) -> String {
        let head = crypto::b64_url(br#"{"alg":"HS256","typ":"JWT"}"#);
        let claims =
            Claims { sub: u.id.clone(), role: u.role.clone(), pwdv: u.pwd_ver, iat: now, exp: now + TOKEN_TTL_SECS };
        let body = crypto::b64_url(&serde_json::to_vec(&claims).expect("claims serialize"));
        let signing = format!("{head}.{body}");
        let sig = crypto::b64_url(&crypto::hmac_sha256(&self.secret, signing.as_bytes()));
        format!("{signing}.{sig}")
    }

    /// Checks signature, expiry, liveness and password version against
    /// live state; the role returned is the account's current one.
    pub fn verify(&self, token: &str) -> Result<Verified, AuthError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(msg("bad token"));
        }
        let want = crypto::b64_url(&crypto::hmac_sha256(&self.secret, format!("{}.{}", parts[0], parts[1]).as_bytes()));
        if !crypto::ct_eq(want.as_bytes(), parts[2].as_bytes()) {
            return Err(msg("bad token"));
        }
        let raw = crypto::b64_url_decode(parts[1]).ok_or_else(|| msg("bad token"))?;
        let c: Claims = serde_json::from_slice(&raw).map_err(|_| msg("bad token"))?;
        if lain_core::now_unix() > c.exp {
            return Err(msg("token expired"));
        }
        let u = self.get(&c.sub).ok().flatten().ok_or_else(|| msg("unknown subject"))?;
        if u.disabled {
            return Err(msg("account disabled"));
        }
        if u.pwd_ver != c.pwdv {
            return Err(msg("password rotated"));
        }
        Ok(Verified { user_id: u.id, role: u.role })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: std::path::PathBuf,
        store: Option<Store>,
        secret: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = lain_store::test_dir("auth");
            let store = Store::open(&dir).unwrap();
            let secret = load_or_create_secret(&store).unwrap();
            Fixture { dir, store: Some(store), secret }
        }
        fn auth(&self) -> Auth<'_> {
            Auth::new(self.store.as_ref().unwrap(), self.secret.clone())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.store.take();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn setup_login_verify() {
        let f = Fixture::new();
        let a = f.auth();
        assert!(!a.has_users());
        assert_eq!(a.setup("admin", "short").unwrap_err().to_string(), "username required, password >= 8 chars");
        let u = a.setup("admin", "password123").unwrap();
        assert_eq!(
            (u.id.as_str(), u.role.as_str(), u.pwd_ver, u.pass_hash.is_none()),
            ("user-admin", "admin", 1, true)
        );
        assert_eq!(a.setup("x", "password123").unwrap_err().to_string(), "setup already completed");
        assert_eq!(a.login("admin", "nope-nope").unwrap_err().to_string(), "invalid credentials");
        assert_eq!(a.login("ghost", "password123").unwrap_err().to_string(), "invalid credentials");
        let tok = a.login("admin", "password123").unwrap();
        assert_eq!(a.verify(&tok).unwrap(), Verified { user_id: "user-admin".into(), role: "admin".into() });
        assert!(a.verify("a.b").is_err());
        let mut forged = tok.clone();
        forged.push('x');
        assert!(a.verify(&forged).is_err());
    }

    #[test]
    fn password_change_kills_tokens_and_disable_locks_out() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        a.create("bob", "bobpassword", "user").unwrap();
        let tok = a.login("bob", "bobpassword").unwrap();
        assert_eq!(
            a.change_password("user-bob", "wrong-pass", "newpassword").unwrap_err().to_string(),
            "invalid credentials"
        );
        assert_eq!(
            a.change_password("user-bob", "bobpassword", "short").unwrap_err().to_string(),
            "password >= 8 chars"
        );
        a.change_password("user-bob", "bobpassword", "newpassword").unwrap();
        assert_eq!(a.verify(&tok).unwrap_err().to_string(), "password rotated");
        let tok = a.login("bob", "newpassword").unwrap();
        a.set_disabled("user-bob", true).unwrap();
        assert_eq!(a.verify(&tok).unwrap_err().to_string(), "account disabled");
        assert_eq!(a.login("bob", "newpassword").unwrap_err().to_string(), "account disabled");
        a.set_disabled("user-bob", false).unwrap();
        a.set_role("user-bob", "admin").unwrap();
        assert_eq!(a.verify(&tok).unwrap().role, "admin", "role is read live");
        assert_eq!(a.set_disabled("user-ghost", true).unwrap_err(), AuthError::UnknownUser);
    }

    #[test]
    fn expired_tokens_fail() {
        let f = Fixture::new();
        let a = f.auth();
        let u = a.setup("admin", "password123").unwrap();
        let stored = a.get(&u.id).unwrap().unwrap();
        let tok = a.mint(&stored, lain_core::now_unix() - TOKEN_TTL_SECS - 5);
        assert_eq!(a.verify(&tok).unwrap_err().to_string(), "token expired");
    }

    #[test]
    fn create_and_list() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        assert_eq!(a.create("bob", "bobpassword", "root").unwrap_err().to_string(), "unknown role");
        a.create("carol", "carolpassword", "user").unwrap();
        a.create("bob", "bobpassword", "user").unwrap();
        assert_eq!(a.create("bob", "bobpassword", "user").unwrap_err().to_string(), "username taken");
        let names: Vec<_> = a.list().unwrap().into_iter().map(|u| u.username).collect();
        assert_eq!(names, vec!["admin", "bob", "carol"]);
    }

    #[test]
    fn profile_and_preferences() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        let u = a
            .update_profile(
                "user-admin",
                &ProfilePatch {
                    display_name: Some("  The   Admin ".into()),
                    bio: Some("hi\nthere".into()),
                    mascot: Some("moth".into()),
                },
            )
            .unwrap();
        assert_eq!(u.profile.display_name, "The Admin");
        assert_eq!(u.profile.bio, "hi\nthere");
        assert_eq!(u.profile.avatar.mascot, "moth");
        let e = a
            .update_profile("user-admin", &ProfilePatch { mascot: Some("nope".into()), ..Default::default() })
            .unwrap_err();
        assert_eq!(e.to_string(), "unknown mascot");
        let e = a
            .update_profile("user-admin", &ProfilePatch { display_name: Some("a\nb".into()), ..Default::default() })
            .unwrap_err();
        assert_eq!(e.to_string(), "display_name must not contain control characters");
        let e = a
            .update_profile("user-admin", &ProfilePatch { bio: Some("x".repeat(161)), ..Default::default() })
            .unwrap_err();
        assert_eq!(e.to_string(), "bio is too long");
        a.set_preferred_language("user-admin", " JPN ").unwrap();
        assert_eq!(a.get_public("user-admin").unwrap().preferred_language, "jpn");
        assert!(a.set_preferred_language("user-admin", "jp").is_err());
        assert!(a.set_preferred_language("user-admin", "j1n").is_err());
        let e = a.set_playback("user-admin", PlaybackPolicy { max_streams: 40, ..Default::default() }).unwrap_err();
        assert_eq!(e.to_string(), "max_streams must be 0..32");
    }

    #[test]
    fn go_records_and_hashes_verify() {
        // A user record exactly as the Go server stores it (bcrypt
        // `$2a$10$`, Go []byte as base64), for password "password123".
        let f = Fixture::new();
        let real = hash_password("password123").unwrap();
        assert!(real.starts_with("$2a$10$"));
        let raw = format!(
            r#"{{"id":"user-admin","username":"admin","pass_hash":"{}","role":"admin","disabled":false,"pwd_ver":3,"created_at":1,"playback":{{}},"profile":{{"avatar":{{}}}}}}"#,
            crypto::b64_std(real.as_bytes())
        );
        f.store
            .as_ref()
            .unwrap()
            .write(|tx| {
                tx.put(Bucket::Users, b"user-admin", raw.as_bytes())?;
                tx.put(Bucket::UsersByName, b"admin", b"user-admin")
            })
            .unwrap();
        let a = f.auth();
        let tok = a.login("admin", "password123").unwrap();
        assert_eq!(a.verify(&tok).unwrap().user_id, "user-admin");
        // $2b$ (other implementations) verifies too.
        assert!(verify_password("password123", &real.replacen("$2a$", "$2b$", 1)));
        assert!(!verify_password(&"x".repeat(73), &real));
        assert!(hash_password(&"x".repeat(73)).is_err());
    }
}
