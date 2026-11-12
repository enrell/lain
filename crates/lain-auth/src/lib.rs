//! Accounts and session tokens, the Rust port of `internal/auth`.
//!
//! First-run setup creates the admin; admins create users. Passwords
//! and library PINs are Argon2id PHC strings (D-177). Tokens are HS256
//! JWTs signed with the store's secret, each with a random `jti` (its
//! session), and checked on every request against live state:
//! signature, expiry, account liveness and password version. A password change bumps the version, so every
//! earlier token dies on its next use; disabling an account locks it
//! out within one request.

pub mod crypto;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};

use lain_store::{Bucket, Store, StoreError};

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_USER: &str = "user";

/// Token lifetime.
pub const TOKEN_TTL_SECS: i64 = 24 * 3600;

/// The avatar kind of an uploaded picture.
pub const AVATAR_UPLOAD: &str = "upload";

/// The pre-made avatar set (D-086). The artwork lives in the clients.
pub const MASCOTS: [&str; 8] = ["wired", "moth", "static", "orbit", "glyph", "shell", "neon", "void"];

const MAX_DISPLAY_NAME: usize = 40;
const MAX_BIO: usize = 160;

pub use lain_api::{Avatar, PlaybackPolicy, PreferencesPatch, Profile, ProfilePatch, SUBTITLE_MODES, TrackPreferences};

/// A local account. `pass_hash` is an Argon2id PHC string; `null` in
/// every response.
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
    /// D-071's one language. Since D-280 every write keeps it equal to
    /// [`TrackPreferences::legacy_language`] of the split fields, so an
    /// older client reading it gets the closest single answer.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_language: String,
    /// D-280's split preferences. A record with none of the three was
    /// written before D-280 (or never set any) and reads as
    /// `preferred_language` for both languages, subtitles `auto`
    /// ([`TrackPreferences::read`]); no migration pass rewrites old
    /// records, the first preferences write does. `auto` is stored as
    /// an empty mode, so a fresh account stores and answers nothing new.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_audio_language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_subtitle_language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_subtitle_mode: String,
    #[serde(default)]
    pub playback: PlaybackPolicy,
    #[serde(default)]
    pub profile: Profile,
}

impl User {
    /// The record without its hash. A record from before D-280 answers
    /// its one language as both split ones too, so a client reads the
    /// D-280 fields alone (D-280).
    pub fn public(mut self) -> User {
        self.pass_hash = None;
        let t = self.track_preferences();
        self.set_tracks(&t);
        self
    }

    /// The account's track preferences (D-280), read the same way for
    /// old and new records.
    pub fn track_preferences(&self) -> TrackPreferences {
        TrackPreferences::read(
            &self.preferred_language,
            &self.preferred_audio_language,
            &self.preferred_subtitle_language,
            &self.preferred_subtitle_mode,
        )
    }

    /// Stores `t` in the record's shape, D-071's language included.
    fn set_tracks(&mut self, t: &TrackPreferences) {
        self.preferred_audio_language = t.audio_language.clone();
        self.preferred_subtitle_language = t.subtitle_language.clone();
        self.preferred_subtitle_mode = if t.subtitle_mode == "auto" { String::new() } else { t.subtitle_mode.clone() };
        self.preferred_language = t.legacy_language().to_string();
    }

    fn password_ok(&self, password: &str) -> bool {
        self.pass_hash.as_deref().is_some_and(|h| verify_password(password, h))
    }
}

/// An authenticated identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub user_id: String,
    pub role: String,
    /// Identifies this token (a login session): unlocks of private
    /// libraries are kept per session (D-156).
    pub session: String,
    /// When the token stops verifying.
    pub expires_at: i64,
}

impl Verified {
    pub fn is_admin(&self) -> bool {
        self.role == ROLE_ADMIN
    }
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

/// Hashes a password or PIN: Argon2id with the crate's default
/// parameters (19 MiB, 2 passes, 1 lane) and a random 16-byte salt, as
/// a PHC string (`$argon2id$v=19$...`).
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let hash: PasswordHash =
        Argon2::default().hash_password(password.as_bytes()).map_err(|e| AuthError::Msg(format!("hash: {e}")))?;
    Ok(hash.to_string())
}

/// Checks a password against a PHC string; the parameters come from
/// the string, so hashes made with other parameters keep verifying.
pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default().verify_password(password.as_bytes(), hash).is_ok()
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    role: String,
    pwdv: u64,
    iat: i64,
    exp: i64,
    /// A random id per login: the session. Two logins in the same
    /// second are two sessions (private-library unlocks are per
    /// session, D-156).
    jti: String,
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

/// The token secret length in bytes.
const SECRET_LEN: usize = 32;

/// Loads the token secret (`meta/secret`, 32 random bytes), generating
/// it once.
pub fn load_or_create_secret(store: &Store) -> Result<Vec<u8>, AuthError> {
    let secret = store.write(|tx| {
        if let Some(raw) = tx.get(Bucket::Meta, b"secret")? {
            return Ok(raw);
        }
        let mut fresh = vec![0u8; SECRET_LEN];
        crypto::random_bytes(&mut fresh).map_err(|e| StoreError::Abort(format!("secret: {e}")))?;
        tx.put(Bucket::Meta, b"secret", &fresh)?;
        Ok(fresh)
    })?;
    if secret.len() != SECRET_LEN {
        return Err(msg("auth: corrupt secret"));
    }
    Ok(secret)
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
            .map_err(|e| match fail.take() {
                Some(f) => f,
                None => AuthError::from(e),
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
            pass_hash: Some(hash),
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

    /// Sets D-071's one language (an older client): both D-280
    /// languages become it; empty clears both.
    pub fn set_preferred_language(&self, id: &str, language: &str) -> Result<(), AuthError> {
        let patch = PreferencesPatch { preferred_language: Some(language.into()), ..PreferencesPatch::default() };
        self.update_preferences(id, &patch).map(|_| ())
    }

    /// Applies a preferences patch (D-071, D-280) and returns the public
    /// record. Languages are ISO 639-2 codes, lowercased; empty clears.
    /// `preferred_language` sets both languages and leaves the subtitle
    /// mode alone; the D-280 fields present then override it. Nothing
    /// is written when any value is refused.
    pub fn update_preferences(&self, id: &str, p: &PreferencesPatch) -> Result<User, AuthError> {
        let language = |v: &Option<String>, field: &str| -> Result<Option<String>, AuthError> {
            let Some(raw) = v else { return Ok(None) };
            let l = raw.trim().to_lowercase();
            if !l.is_empty() && (l.len() != 3 || !l.bytes().all(|c| c.is_ascii_lowercase())) {
                return Err(AuthError::Msg(format!("{field} must be a three-letter ISO 639-2 code")));
            }
            Ok(Some(l))
        };
        let both = language(&p.preferred_language, "preferred_language")?;
        let audio = language(&p.preferred_audio_language, "preferred_audio_language")?;
        let subtitle = language(&p.preferred_subtitle_language, "preferred_subtitle_language")?;
        let mode = match p.preferred_subtitle_mode.as_deref().map(|m| m.trim().to_lowercase()) {
            None => None,
            Some(m) if m.is_empty() => Some("auto".to_string()),
            Some(m) if SUBTITLE_MODES.contains(&m.as_str()) => Some(m),
            Some(_) => return Err(msg("preferred_subtitle_mode must be auto, always, forced or off")),
        };
        self.update(id, |u| {
            let mut t = u.track_preferences();
            if let Some(l) = both {
                t.audio_language = l.clone();
                t.subtitle_language = l;
            }
            if let Some(l) = audio {
                t.audio_language = l;
            }
            if let Some(l) = subtitle {
                t.subtitle_language = l;
            }
            if let Some(m) = mode {
                t.subtitle_mode = m;
            }
            u.set_tracks(&t);
            Ok(())
        })
        .map(User::public)
    }

    /// The account's track preferences (D-280); the defaults for an
    /// unknown account.
    pub fn track_preferences(&self, id: &str) -> TrackPreferences {
        self.get(id).ok().flatten().map(|u| u.track_preferences()).unwrap_or_default()
    }

    pub fn playback_policy(&self, id: &str) -> PlaybackPolicy {
        self.get(id).ok().flatten().map(|u| u.playback).unwrap_or_default()
    }

    /// Verifies the old password, sets the new one and bumps the
    /// password version.
    pub fn change_password(&self, id: &str, old: &str, new: &str) -> Result<(), AuthError> {
        let u = self.get(id)?.ok_or(AuthError::UnknownUser)?;
        if !u.password_ok(old) {
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
        u.pass_hash = Some(hash_password(new)?);
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

    /// Marks the account as using its uploaded picture (the caller has
    /// already stored the file); the new version makes a new URL.
    pub fn set_uploaded_avatar(&self, id: &str) -> Result<User, AuthError> {
        self.update(id, |u| {
            u.profile.avatar =
                Avatar { kind: AVATAR_UPLOAD.into(), mascot: String::new(), version: lain_core::now_unix_millis() };
            Ok(())
        })
        .map(User::public)
    }

    /// Checks a user's password (re-authentication for PIN changes and
    /// library unlocks).
    pub fn check_password(&self, id: &str, password: &str) -> bool {
        self.get(id).ok().flatten().is_some_and(|u| u.password_ok(password))
    }

    /// Sets the user's library PIN (4 to 12 digits) after checking the
    /// password. The PIN is Argon2id-hashed in `user_pins` (D-156).
    pub fn set_pin(&self, id: &str, pin: &str, password: &str) -> Result<(), AuthError> {
        if !self.check_password(id, password) {
            return Err(msg("invalid credentials"));
        }
        if !(4..=12).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
            return Err(msg("pin must be 4 to 12 digits"));
        }
        let hash = hash_password(pin)?;
        self.store.write(|tx| tx.put_json(Bucket::UserPins, id.as_bytes(), &serde_json::json!({"hash": hash})))?;
        Ok(())
    }

    /// Removes the user's PIN after checking the password.
    pub fn clear_pin(&self, id: &str, password: &str) -> Result<(), AuthError> {
        if !self.check_password(id, password) {
            return Err(msg("invalid credentials"));
        }
        self.store.write(|tx| tx.delete(Bucket::UserPins, id.as_bytes()))?;
        Ok(())
    }

    pub fn has_pin(&self, id: &str) -> bool {
        self.store.read(|tx| tx.get(Bucket::UserPins, id.as_bytes())).ok().flatten().is_some()
    }

    /// Checks a PIN; false when none is set.
    pub fn check_pin(&self, id: &str, pin: &str) -> bool {
        let rec: Option<serde_json::Value> =
            self.store.read(|tx| tx.get_json(Bucket::UserPins, id.as_bytes())).ok().flatten();
        rec.and_then(|r| r["hash"].as_str().map(str::to_string)).is_some_and(|h| verify_password(pin, &h))
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
        if !u.password_ok(password) {
            return Err(invalid());
        }
        self.mint(&u, lain_core::now_unix())
    }

    /// Signs a token for `u` issued at `now`, with a fresh session id.
    pub fn mint(&self, u: &User, now: i64) -> Result<String, AuthError> {
        let head = crypto::b64_url(br#"{"alg":"HS256","typ":"JWT"}"#);
        let mut id = [0u8; 12];
        crypto::random_bytes(&mut id).map_err(|e| AuthError::Msg(format!("token: {e}")))?;
        let jti = crypto::b64_url(&id);
        let claims = Claims {
            sub: u.id.clone(),
            role: u.role.clone(),
            pwdv: u.pwd_ver,
            iat: now,
            exp: now + TOKEN_TTL_SECS,
            jti,
        };
        let body = crypto::b64_url(&serde_json::to_vec(&claims).expect("claims serialize"));
        let signing = format!("{head}.{body}");
        let sig = crypto::b64_url(&crypto::hmac_sha256(&self.secret, signing.as_bytes()));
        Ok(format!("{signing}.{sig}"))
    }

    /// Checks signature, expiry, liveness and password version against
    /// live state; the role returned is the account's current one.
    pub fn verify(&self, token: &str) -> Result<Verified, AuthError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(msg("bad token"));
        }
        let tag = crypto::b64_url_decode(parts[2]).ok_or_else(|| msg("bad token"))?;
        if !crypto::hmac_sha256_verify(&self.secret, format!("{}.{}", parts[0], parts[1]).as_bytes(), &tag) {
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
        if c.jti.is_empty() {
            return Err(msg("bad token"));
        }
        Ok(Verified { user_id: u.id, role: u.role, session: c.jti, expires_at: c.exp })
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
        let v = a.verify(&tok).unwrap();
        assert_eq!((v.user_id.as_str(), v.role.as_str(), v.session.len()), ("user-admin", "admin", 16));
        assert!(v.expires_at > lain_core::now_unix());
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
    fn pins() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        assert!(!a.has_pin("user-admin"));
        assert_eq!(a.set_pin("user-admin", "1234", "wrong-pass").unwrap_err().to_string(), "invalid credentials");
        assert_eq!(
            a.set_pin("user-admin", "12a4", "password123").unwrap_err().to_string(),
            "pin must be 4 to 12 digits"
        );
        assert!(a.set_pin("user-admin", "123", "password123").is_err());
        a.set_pin("user-admin", "1234", "password123").unwrap();
        assert!(a.has_pin("user-admin") && a.check_pin("user-admin", "1234") && !a.check_pin("user-admin", "4321"));
        assert!(!a.check_pin("user-nobody", "1234"));
        a.clear_pin("user-admin", "password123").unwrap();
        assert!(!a.check_pin("user-admin", "1234"));
    }

    #[test]
    fn sessions_differ_per_token() {
        let f = Fixture::new();
        let a = f.auth();
        let u = a.setup("admin", "password123").unwrap();
        let stored = a.get(&u.id).unwrap().unwrap();
        let now = lain_core::now_unix();
        let (t1, t2) = (a.mint(&stored, now).unwrap(), a.mint(&stored, now).unwrap());
        assert_ne!(a.verify(&t1).unwrap().session, a.verify(&t2).unwrap().session);
        assert_eq!(a.verify(&t1).unwrap().session, a.verify(&t1).unwrap().session);
    }

    #[test]
    fn expired_tokens_fail() {
        let f = Fixture::new();
        let a = f.auth();
        let u = a.setup("admin", "password123").unwrap();
        let stored = a.get(&u.id).unwrap().unwrap();
        let tok = a.mint(&stored, lain_core::now_unix() - TOKEN_TTL_SECS - 5).unwrap();
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

    fn prefs(audio: &str, subtitle: &str, mode: &str) -> TrackPreferences {
        TrackPreferences {
            audio_language: audio.into(),
            subtitle_language: subtitle.into(),
            subtitle_mode: mode.into(),
        }
    }

    #[test]
    fn a_record_from_before_d280_reads_as_its_one_language() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        // The shape D-071 stored: one language, none of the D-280 fields.
        let old = br#"{"id":"user-old","username":"old","pass_hash":null,"role":"user","pwd_ver":1,"created_at":1,"preferred_language":"jpn","playback":{},"profile":{}}"#;
        f.store
            .as_ref()
            .unwrap()
            .write(|tx| {
                tx.put(Bucket::Users, b"user-old", old)?;
                tx.put(Bucket::UsersByName, b"old", b"user-old")
            })
            .unwrap();
        assert_eq!(a.track_preferences("user-old"), prefs("jpn", "jpn", "auto"));
        // Answered with both split fields, so a client reads those alone.
        let u = a.get_public("user-old").unwrap();
        assert_eq!(
            (
                u.preferred_language.as_str(),
                u.preferred_audio_language.as_str(),
                u.preferred_subtitle_language.as_str(),
                u.preferred_subtitle_mode.as_str()
            ),
            ("jpn", "jpn", "jpn", "")
        );
        // Another write to the record keeps it old-shaped and reading the same.
        a.set_disabled("user-old", false).unwrap();
        let raw: serde_json::Value =
            f.store.as_ref().unwrap().read(|tx| tx.get_json(Bucket::Users, b"user-old")).unwrap().unwrap();
        assert!(raw.get("preferred_audio_language").is_none(), "{raw}");
        assert_eq!(a.track_preferences("user-old"), prefs("jpn", "jpn", "auto"));
        // A fresh account and an unknown one have no preference at all.
        assert_eq!(a.track_preferences("user-admin"), prefs("", "", "auto"));
        assert_eq!(a.track_preferences("user-ghost"), TrackPreferences::default());
    }

    #[test]
    fn split_preferences_and_the_one_language_of_older_clients() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        let patch = |audio: Option<&str>, subtitle: Option<&str>, mode: Option<&str>| PreferencesPatch {
            preferred_audio_language: audio.map(str::to_string),
            preferred_subtitle_language: subtitle.map(str::to_string),
            preferred_subtitle_mode: mode.map(str::to_string),
            ..PreferencesPatch::default()
        };
        // Japanese audio, Portuguese subtitles: an older client reads the
        // language the viewer reads.
        let u = a.update_preferences("user-admin", &patch(Some(" JPN "), Some("por"), None)).unwrap();
        assert_eq!(u.track_preferences(), prefs("jpn", "por", "auto"));
        assert_eq!(u.preferred_language, "por");
        // Only the fields present change.
        let u = a.update_preferences("user-admin", &patch(None, None, Some("Forced"))).unwrap();
        assert_eq!(u.track_preferences(), prefs("jpn", "por", "forced"));
        let u = a.update_preferences("user-admin", &patch(None, Some(""), None)).unwrap();
        assert_eq!((u.track_preferences(), u.preferred_language.as_str()), (prefs("jpn", "", "forced"), "jpn"));
        // An older client's one language sets both and leaves the mode.
        a.set_preferred_language("user-admin", "eng").unwrap();
        assert_eq!(a.track_preferences("user-admin"), prefs("eng", "eng", "forced"));
        // Both at once: the old field first, the new ones over it.
        let both = PreferencesPatch {
            preferred_language: Some("por".into()),
            preferred_audio_language: Some("jpn".into()),
            ..PreferencesPatch::default()
        };
        assert_eq!(
            a.update_preferences("user-admin", &both).unwrap().track_preferences(),
            prefs("jpn", "por", "forced")
        );
        // An empty mode is auto again; clearing both clears the old field.
        let u = a.update_preferences("user-admin", &patch(Some(""), Some(""), Some(""))).unwrap();
        assert_eq!((u.track_preferences(), u.preferred_language.as_str()), (prefs("", "", "auto"), ""));
        // Refusals name the field and write nothing.
        a.update_preferences("user-admin", &patch(Some("jpn"), Some("por"), Some("off"))).unwrap();
        for (p, e) in [
            (patch(Some("ja"), Some("eng"), None), "preferred_audio_language must be a three-letter ISO 639-2 code"),
            (
                patch(Some("eng"), Some("p0r"), None),
                "preferred_subtitle_language must be a three-letter ISO 639-2 code",
            ),
            (
                patch(Some("eng"), None, Some("sometimes")),
                "preferred_subtitle_mode must be auto, always, forced or off",
            ),
        ] {
            assert_eq!(a.update_preferences("user-admin", &p).unwrap_err().to_string(), e);
        }
        assert_eq!(a.track_preferences("user-admin"), prefs("jpn", "por", "off"));
        assert_eq!(
            a.update_preferences("user-ghost", &patch(Some("jpn"), None, None)).unwrap_err(),
            AuthError::UnknownUser
        );
    }

    #[test]
    fn hashes_are_argon2id_phc_strings() {
        let h = hash_password("password123").unwrap();
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        assert_ne!(h, hash_password("password123").unwrap(), "every hash has its own salt");
        assert!(verify_password("password123", &h));
        assert!(!verify_password("password124", &h));
        assert!(!verify_password("password123", "not a phc string"));
        // No length cap: long passphrases hash and verify whole.
        let long = "x".repeat(200);
        let lh = hash_password(&long).unwrap();
        assert!(verify_password(&long, &lh) && !verify_password(&long[..72], &lh));
        // The stored record holds the PHC string as is.
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        assert!(a.get("user-admin").unwrap().unwrap().pass_hash.unwrap().starts_with("$argon2id$"));
    }

    #[test]
    fn tokens_without_a_session_id_are_refused() {
        let f = Fixture::new();
        let a = f.auth();
        a.setup("admin", "password123").unwrap();
        let now = lain_core::now_unix();
        let head = crypto::b64_url(br#"{"alg":"HS256","typ":"JWT"}"#);
        let body = crypto::b64_url(
            format!(r#"{{"sub":"user-admin","role":"admin","pwdv":1,"iat":{now},"exp":{}}}"#, now + 60).as_bytes(),
        );
        let signing = format!("{head}.{body}");
        let tok = format!("{signing}.{}", crypto::b64_url(&crypto::hmac_sha256(&f.secret, signing.as_bytes())));
        assert_eq!(a.verify(&tok).unwrap_err().to_string(), "bad token");
    }
}
