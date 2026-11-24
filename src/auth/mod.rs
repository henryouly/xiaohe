use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng as ArgonOsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

use crate::config::{
    AllowedAreas, ApiTokenConfig, ConfigError, ConfigStore, UserConfig, UserRole, UsersConfig,
};
use crate::protocol::domain::{DomainState, EntityRecord};

pub const SESSION_COOKIE_NAME: &str = "xiaohe_session";
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("system is already initialized with an admin user")]
    AlreadyInitialized,
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("username '{0}' already exists")]
    UserExists(String),
    #[error("user '{0}' not found")]
    UserNotFound(String),
    #[error("token '{0}' not found")]
    TokenNotFound(String),
    #[error("cannot remove or demote the last remaining admin user")]
    LastAdminProtected,
    #[error("validation error: {0}")]
    Validation(String),
    #[error("config persistence error: {0}")]
    Config(#[from] ConfigError),
    #[error("password hashing error: {0}")]
    Hash(String),
}

/// Public user representation returned by API endpoints (never includes hashes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: UserRole,
    pub allowed_areas: AllowedAreas,
    pub allow_guest_locks: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_tokens: Vec<ApiTokenSummary>,
}

impl From<&UserConfig> for UserSummary {
    fn from(u: &UserConfig) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            display_name: u.display_name.clone(),
            role: u.role,
            allowed_areas: u.allowed_areas.clone(),
            allow_guest_locks: u.allow_guest_locks,
            api_tokens: u
                .api_tokens
                .iter()
                .map(|t| ApiTokenSummary {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    created_at: t.created_at,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiTokenSummary {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintedApiToken {
    pub id: String,
    pub name: String,
    pub token: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct CreateUserInput {
    pub username: String,
    pub display_name: String,
    pub password: String,
    pub role: UserRole,
    pub allowed_areas: AllowedAreas,
    pub allow_guest_locks: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateUserInput {
    pub display_name: Option<String>,
    pub password: Option<String>,
    pub role: Option<UserRole>,
    pub allowed_areas: Option<AllowedAreas>,
    pub allow_guest_locks: Option<bool>,
}

/// Credential presented by an authenticated caller, used to re-validate live RBAC
/// permissions and revocation on long-lived streams such as SSE (`GET /api/events`).
#[derive(Debug, Clone)]
pub enum AuthCredential {
    Session(String),
    Bearer(String),
}

/// Authenticated caller context extracted by Axum middleware/extractors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: UserRole,
    pub allowed_areas: AllowedAreas,
    pub allow_guest_locks: bool,
}

impl AuthenticatedUser {
    pub fn is_admin(&self) -> bool {
        self.role == UserRole::Admin
    }

    /// Check whether this caller is permitted to view/access entities in `area_id`.
    ///
    /// Admins see everything (including unassigned entities where `area_id` is `None`).
    /// Area-restricted Members/Guests only see entities explicitly assigned to one of their
    /// `allowed_areas`.
    pub fn can_access_area(&self, area_id: Option<&str>) -> bool {
        if self.is_admin() {
            return true;
        }
        match &self.allowed_areas {
            AllowedAreas::All => true,
            AllowedAreas::List(areas) => area_id.is_some_and(|a| areas.iter().any(|x| x == a)),
        }
    }

    /// Check whether this caller is permitted to issue `action` on `entity`.
    pub fn can_command_entity(&self, entity: &EntityRecord, _action: &str) -> bool {
        if !self.can_access_area(entity.area_id.as_deref()) {
            return false;
        }
        if self.role == UserRole::Guest
            && matches!(entity.state, DomainState::Lock { .. })
            && !self.allow_guest_locks
        {
            return false;
        }
        true
    }
}

impl From<&UserConfig> for AuthenticatedUser {
    fn from(u: &UserConfig) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            display_name: u.display_name.clone(),
            role: u.role,
            allowed_areas: u.allowed_areas.clone(),
            allow_guest_locks: u.allow_guest_locks,
        }
    }
}

struct SessionEntry {
    user_id: String,
    expires_at_ms: i64,
}

struct AuthInner {
    config_store: ConfigStore,
    users: RwLock<UsersConfig>,
    sessions: RwLock<HashMap<String, SessionEntry>>,
    session_ttl: Duration,
}

/// Multi-user Argon2id authentication, session store, API token store, and RBAC policy engine.
#[derive(Clone)]
pub struct AuthService {
    inner: Arc<AuthInner>,
}

impl AuthService {
    pub fn new(config_store: ConfigStore, users: UsersConfig) -> Self {
        Self::with_session_ttl(config_store, users, DEFAULT_SESSION_TTL)
    }

    pub fn with_session_ttl(
        config_store: ConfigStore,
        users: UsersConfig,
        session_ttl: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(AuthInner {
                config_store,
                users: RwLock::new(users),
                sessions: RwLock::new(HashMap::new()),
                session_ttl,
            }),
        }
    }

    /// Returns `true` once at least one Admin user exists.
    pub fn is_initialized(&self) -> bool {
        self.inner
            .users
            .read()
            .expect("users rwlock poisoned")
            .users
            .iter()
            .any(|u| u.role == UserRole::Admin)
    }

    /// First-boot only: creates the initial Admin account and returns its summary + session token.
    pub fn bootstrap_admin(
        &self,
        username: &str,
        password: &str,
        display_name: &str,
    ) -> Result<(UserSummary, String), AuthError> {
        let username = validate_username(username)?;
        validate_password(password)?;
        let display_name = if display_name.trim().is_empty() {
            username.clone()
        } else {
            display_name.trim().to_owned()
        };

        let password_hash = hash_password(password)?;
        let user = UserConfig {
            id: format!("usr_{}", uuid::Uuid::now_v7().simple()),
            username,
            display_name,
            password_hash,
            role: UserRole::Admin,
            allowed_areas: AllowedAreas::All,
            allow_guest_locks: false,
            api_tokens: Vec::new(),
        };

        {
            let mut guard = self.inner.users.write().expect("users rwlock poisoned");
            if guard.users.iter().any(|u| u.role == UserRole::Admin) {
                return Err(AuthError::AlreadyInitialized);
            }
            let mut candidate = guard.clone();
            candidate.users.push(user.clone());
            self.inner.config_store.save_users(&candidate)?;
            *guard = candidate;
        }

        let sid = self.create_session(&user.id);
        Ok((UserSummary::from(&user), sid))
    }

    /// Authenticate username/password using Argon2id and create a new session cookie token.
    pub fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<(UserSummary, String), AuthError> {
        let user = {
            let guard = self.inner.users.read().expect("users rwlock poisoned");
            guard
                .users
                .iter()
                .find(|u| u.username.eq_ignore_ascii_case(username.trim()))
                .cloned()
        }
        .ok_or(AuthError::InvalidCredentials)?;

        if !verify_password(password, &user.password_hash) {
            return Err(AuthError::InvalidCredentials);
        }

        // Re-check under the users read lock that the account still exists with the exact
        // password hash we verified, and insert the session before releasing the lock.
        // Because update_user/delete_user hold the users write lock while revoking sessions,
        // an in-flight login using an old password can never mint a surviving session.
        let guard = self.inner.users.read().expect("users rwlock poisoned");
        let current = guard
            .users
            .iter()
            .find(|u| u.id == user.id)
            .ok_or(AuthError::InvalidCredentials)?;
        if current.password_hash != user.password_hash {
            return Err(AuthError::InvalidCredentials);
        }
        let sid = self.create_session(&current.id);
        Ok((UserSummary::from(current), sid))
    }

    /// Re-validate a previously extracted credential against current sessions, tokens, and RBAC state.
    pub fn revalidate(&self, credential: &AuthCredential) -> Option<AuthenticatedUser> {
        match credential {
            AuthCredential::Session(sid) => self.authenticate_session(sid),
            AuthCredential::Bearer(token) => self.authenticate_bearer(token),
        }
    }

    /// Revoke an active session cookie token.
    pub fn logout(&self, session_id: &str) {
        self.inner
            .sessions
            .write()
            .expect("sessions rwlock poisoned")
            .remove(session_id);
    }

    /// Resolve an active `xiaohe_session` cookie into an `AuthenticatedUser`.
    pub fn authenticate_session(&self, session_id: &str) -> Option<AuthenticatedUser> {
        let now = now_unix_ms();
        let user_id = {
            let guard = self
                .inner
                .sessions
                .read()
                .expect("sessions rwlock poisoned");
            let entry = guard.get(session_id)?;
            if entry.expires_at_ms <= now {
                None
            } else {
                Some(entry.user_id.clone())
            }
        };

        let Some(user_id) = user_id else {
            self.logout(session_id);
            return None;
        };

        let guard = self.inner.users.read().expect("users rwlock poisoned");
        guard
            .users
            .iter()
            .find(|u| u.id == user_id)
            .map(AuthenticatedUser::from)
    }

    /// Resolve an `Authorization: Bearer <token>` header using constant-time SHA-256 verification.
    pub fn authenticate_bearer(&self, raw_token: &str) -> Option<AuthenticatedUser> {
        let candidate_digest = Sha256::digest(raw_token.as_bytes());
        let guard = self.inner.users.read().expect("users rwlock poisoned");
        for user in &guard.users {
            for tok in &user.api_tokens {
                if let Some(expected) = decode_sha256_hex(&tok.token_hash)
                    && bool::from(candidate_digest.as_slice().ct_eq(&expected))
                {
                    return Some(AuthenticatedUser::from(user));
                }
            }
        }
        None
    }

    pub fn list_users(&self) -> Vec<UserSummary> {
        self.inner
            .users
            .read()
            .expect("users rwlock poisoned")
            .users
            .iter()
            .map(UserSummary::from)
            .collect()
    }

    pub fn get_user_summary(&self, user_id: &str) -> Option<UserSummary> {
        self.inner
            .users
            .read()
            .expect("users rwlock poisoned")
            .users
            .iter()
            .find(|u| u.id == user_id)
            .map(UserSummary::from)
    }

    pub fn create_user(&self, input: CreateUserInput) -> Result<UserSummary, AuthError> {
        let username = validate_username(&input.username)?;
        validate_password(&input.password)?;
        let display_name = if input.display_name.trim().is_empty() {
            username.clone()
        } else {
            input.display_name.trim().to_owned()
        };
        let password_hash = hash_password(&input.password)?;

        let allowed_areas = if input.role == UserRole::Admin {
            AllowedAreas::All
        } else {
            input.allowed_areas
        };

        let user = UserConfig {
            id: format!("usr_{}", uuid::Uuid::now_v7().simple()),
            username: username.clone(),
            display_name,
            password_hash,
            role: input.role,
            allowed_areas,
            allow_guest_locks: input.allow_guest_locks,
            api_tokens: Vec::new(),
        };

        let mut guard = self.inner.users.write().expect("users rwlock poisoned");
        if guard
            .users
            .iter()
            .any(|u| u.username.eq_ignore_ascii_case(&username))
        {
            return Err(AuthError::UserExists(username));
        }
        let mut candidate = guard.clone();
        candidate.users.push(user.clone());
        self.inner.config_store.save_users(&candidate)?;
        *guard = candidate;
        Ok(UserSummary::from(&user))
    }

    pub fn update_user(
        &self,
        user_id: &str,
        input: UpdateUserInput,
    ) -> Result<UserSummary, AuthError> {
        let new_hash = match &input.password {
            Some(pw) => {
                validate_password(pw)?;
                Some(hash_password(pw)?)
            }
            None => None,
        };

        let mut guard = self.inner.users.write().expect("users rwlock poisoned");
        let mut candidate = guard.clone();
        let admin_count = candidate
            .users
            .iter()
            .filter(|u| u.role == UserRole::Admin)
            .count();
        let user = candidate
            .users
            .iter_mut()
            .find(|u| u.id == user_id)
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_owned()))?;

        let mut invalidate_sessions = false;
        if let Some(new_role) = input.role {
            if user.role == UserRole::Admin && new_role != UserRole::Admin && admin_count <= 1 {
                return Err(AuthError::LastAdminProtected);
            }
            if user.role != new_role {
                let was_guest = user.role == UserRole::Guest;
                user.role = new_role;
                if !was_guest && new_role == UserRole::Guest && input.allow_guest_locks.is_none() {
                    user.allow_guest_locks = false;
                }
                invalidate_sessions = true;
            }
        }
        if let Some(name) = input.display_name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                user.display_name = trimmed.to_owned();
            }
        }
        if let Some(areas) = input.allowed_areas {
            user.allowed_areas = if user.role == UserRole::Admin {
                AllowedAreas::All
            } else {
                areas
            };
        }
        if let Some(allow_locks) = input.allow_guest_locks {
            user.allow_guest_locks = allow_locks;
        }
        if let Some(hash) = new_hash {
            user.password_hash = hash;
            invalidate_sessions = true;
        }

        let summary = UserSummary::from(&*user);
        self.inner.config_store.save_users(&candidate)?;
        *guard = candidate;

        if invalidate_sessions {
            self.revoke_user_sessions(user_id);
        }
        Ok(summary)
    }

    pub fn delete_user(&self, user_id: &str) -> Result<(), AuthError> {
        let mut guard = self.inner.users.write().expect("users rwlock poisoned");
        let mut candidate = guard.clone();
        let idx = candidate
            .users
            .iter()
            .position(|u| u.id == user_id)
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_owned()))?;
        let is_admin = candidate.users[idx].role == UserRole::Admin;
        let admin_count = candidate
            .users
            .iter()
            .filter(|u| u.role == UserRole::Admin)
            .count();
        if is_admin && admin_count <= 1 {
            return Err(AuthError::LastAdminProtected);
        }
        candidate.users.remove(idx);
        self.inner.config_store.save_users(&candidate)?;
        *guard = candidate;
        self.revoke_user_sessions(user_id);
        Ok(())
    }

    /// Mint a new API Bearer token for `user_id`; plaintext is returned ONCE and only its
    /// SHA-256 hash is persisted in `config/users.toml`.
    pub fn mint_token(&self, user_id: &str, name: &str) -> Result<MintedApiToken, AuthError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AuthError::Validation("token name must not be empty".into()));
        }

        let raw_token = format!("xh_{}", random_token_b64());
        let token_hash = format!("sha256:{:x}", Sha256::digest(raw_token.as_bytes()));
        let token_id = format!("tok_{}", uuid::Uuid::now_v7().simple());
        let created_at = now_unix_ms();

        {
            let mut guard = self.inner.users.write().expect("users rwlock poisoned");
            let mut candidate = guard.clone();
            let user = candidate
                .users
                .iter_mut()
                .find(|u| u.id == user_id)
                .ok_or_else(|| AuthError::UserNotFound(user_id.to_owned()))?;
            user.api_tokens.push(ApiTokenConfig {
                id: token_id.clone(),
                name: trimmed.to_owned(),
                token_hash,
                created_at,
            });
            self.inner.config_store.save_users(&candidate)?;
            *guard = candidate;
        }

        Ok(MintedApiToken {
            id: token_id,
            name: trimmed.to_owned(),
            token: raw_token,
            created_at,
        })
    }

    pub fn revoke_token(&self, user_id: &str, token_id: &str) -> Result<(), AuthError> {
        let mut guard = self.inner.users.write().expect("users rwlock poisoned");
        let mut candidate = guard.clone();
        let user = candidate
            .users
            .iter_mut()
            .find(|u| u.id == user_id)
            .ok_or_else(|| AuthError::UserNotFound(user_id.to_owned()))?;
        let before = user.api_tokens.len();
        user.api_tokens.retain(|t| t.id != token_id);
        if user.api_tokens.len() == before {
            return Err(AuthError::TokenNotFound(token_id.to_owned()));
        }
        self.inner.config_store.save_users(&candidate)?;
        *guard = candidate;
        Ok(())
    }

    fn create_session(&self, user_id: &str) -> String {
        let sid = random_token_b64();
        let expires_at_ms = now_unix_ms().saturating_add(self.inner.session_ttl.as_millis() as i64);
        self.inner
            .sessions
            .write()
            .expect("sessions rwlock poisoned")
            .insert(
                sid.clone(),
                SessionEntry {
                    user_id: user_id.to_owned(),
                    expires_at_ms,
                },
            );
        sid
    }

    fn revoke_user_sessions(&self, user_id: &str) {
        self.inner
            .sessions
            .write()
            .expect("sessions rwlock poisoned")
            .retain(|_, s| s.user_id != user_id);
    }
}

pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut ArgonOsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Hash(e.to_string()))
}

pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn validate_username(raw: &str) -> Result<String, AuthError> {
    let trimmed = raw.trim();
    if trimmed.len() < 2
        || trimmed.len() > 64
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
    {
        return Err(AuthError::Validation(
            "username must be 2–64 ASCII characters [a-zA-Z0-9_.-]".into(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(AuthError::Validation(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

fn random_token_b64() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_sha256_hex(stored: &str) -> Option<[u8; 32]> {
    let hex = stored
        .strip_prefix("sha256:")
        .or_else(|| stored.strip_prefix("SHA256:"))
        .unwrap_or(stored);
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, &[hi_b, lo_b]) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let hi = hex_nibble(hi_b)?;
        let lo = hex_nibble(lo_b)?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::protocol::domain::{Availability, EntitySource, LockState};

    #[test]
    fn bootstrap_login_token_lifecycle_and_last_admin_protection() {
        let dir = TempDir::new().unwrap();
        let store = ConfigStore::new(dir.path());
        let auth = AuthService::new(store.clone(), UsersConfig::default());

        assert!(!auth.is_initialized());
        let (admin, sid) = auth
            .bootstrap_admin("admin", "supersecure123", "Household Admin")
            .expect("bootstrap");
        assert!(auth.is_initialized());
        assert_eq!(admin.role, UserRole::Admin);

        // Second bootstrap must fail with AlreadyInitialized
        assert!(matches!(
            auth.bootstrap_admin("admin2", "supersecure123", "Second"),
            Err(AuthError::AlreadyInitialized)
        ));

        // Session auth works, logout invalidates it
        assert_eq!(auth.authenticate_session(&sid).unwrap().id, admin.id);
        auth.logout(&sid);
        assert!(auth.authenticate_session(&sid).is_none());

        // Login with wrong password fails; right password succeeds
        assert!(matches!(
            auth.login("admin", "wrongpassword"),
            Err(AuthError::InvalidCredentials)
        ));
        let (logged_in, _) = auth.login("admin", "supersecure123").expect("login");
        assert_eq!(logged_in.id, admin.id);

        // Mint & verify Bearer token, then revoke
        let minted = auth.mint_token(&admin.id, "cli").expect("mint token");
        assert!(minted.token.starts_with("xh_"));
        assert_eq!(
            auth.authenticate_bearer(&minted.token).unwrap().id,
            admin.id
        );
        auth.revoke_token(&admin.id, &minted.id).expect("revoke");
        assert!(auth.authenticate_bearer(&minted.token).is_none());

        // Cannot delete or demote last admin
        assert!(matches!(
            auth.delete_user(&admin.id),
            Err(AuthError::LastAdminProtected)
        ));
        assert!(matches!(
            auth.update_user(
                &admin.id,
                UpdateUserInput {
                    role: Some(UserRole::Member),
                    ..Default::default()
                }
            ),
            Err(AuthError::LastAdminProtected)
        ));

        // Persisted users.toml round-trips cleanly
        let reloaded = store.load_users().expect("reload users");
        assert_eq!(reloaded.users.len(), 1);
        assert_ne!(reloaded.users[0].password_hash, "supersecure123");
    }

    #[test]
    fn rbac_area_scoping_and_guest_lock_policy() {
        let guest = AuthenticatedUser {
            id: "u1".into(),
            username: "guest".into(),
            display_name: "Guest".into(),
            role: UserRole::Guest,
            allowed_areas: AllowedAreas::List(vec!["guest_room".into()]),
            allow_guest_locks: false,
        };
        let lock_in_guest_room = EntityRecord {
            entity_id: "lock.guest_door".into(),
            device_id: None,
            area_id: Some("guest_room".into()),
            name: "Guest Door".into(),
            source: EntitySource::EspHome {
                node_id: "n".into(),
            },
            availability: Availability::Online,
            state: DomainState::Lock {
                state: LockState::Locked,
            },
            capabilities: vec![],
            extra: serde_json::Map::new(),
            custom_actions: vec![],
            last_changed_at: 0,
            last_updated_at: 0,
        };
        let light_in_guest_room = EntityRecord {
            entity_id: "light.guest_lamp".into(),
            state: DomainState::Light {
                on: false,
                brightness: None,
                color_temp_k: None,
                rgb: None,
            },
            ..lock_in_guest_room.clone()
        };

        assert!(guest.can_access_area(Some("guest_room")));
        assert!(!guest.can_access_area(Some("master_bedroom")));
        assert!(!guest.can_access_area(None));
        assert!(guest.can_command_entity(&light_in_guest_room, "turn_on"));
        assert!(!guest.can_command_entity(&lock_in_guest_room, "unlock"));
    }
}
