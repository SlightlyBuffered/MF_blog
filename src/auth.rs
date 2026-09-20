use std::{
    fs,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::{distributions::Alphanumeric, Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::app::AppState;

type HmacSha256 = Hmac<Sha256>;
type HmacSha1 = Hmac<Sha1>;

const SESSION_COOKIE: &str = "mf_blog_session";
const SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthConfig {
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionData {
    pub writer_authenticated: bool,
    pub writer_username: Option<String>,
    pub csrf_token: String,
    pub expires_at: i64,
}

impl SessionData {
    pub fn anonymous() -> Self {
        Self {
            writer_authenticated: false,
            writer_username: None,
            csrf_token: random_token(32),
            expires_at: now_epoch() + SESSION_TTL_SECONDS,
        }
    }

    pub fn authenticated(username: String) -> Self {
        Self {
            writer_authenticated: true,
            writer_username: Some(username),
            csrf_token: random_token(32),
            expires_at: now_epoch() + SESSION_TTL_SECONDS,
        }
    }
}

pub fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn argon2() -> Result<Argon2<'static>, String> {
    let params = Params::new(65_536, 3, 2, None)
        .map_err(|error| format!("invalid Argon2 parameters: {error}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let mut salt_bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| format!("cannot create password salt: {error}"))?;
    argon2()?
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| format!("cannot hash password: {error}"))
}

pub fn verify_password(password_hash: &str, password: &str) -> bool {
    let parsed = match PasswordHash::new(password_hash) {
        Ok(value) => value,
        Err(_) => return false,
    };
    argon2()
        .and_then(|engine| {
            engine
                .verify_password(password.as_bytes(), &parsed)
                .map_err(|error| error.to_string())
        })
        .is_ok()
}

pub fn load_auth_config(path: &Path) -> Result<Option<AuthConfig>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read writer authentication config: {error}")),
    };
    let config: AuthConfig = serde_json::from_str(&content)
        .map_err(|error| format!("writer authentication config is invalid: {error}"))?;
    if config.username.trim().is_empty() || config.password_hash.trim().is_empty() {
        return Err("writer authentication config is invalid".to_string());
    }
    Ok(Some(config))
}

pub fn save_auth_config(path: &Path, username: &str, password: &str) -> Result<(), String> {
    let config = AuthConfig {
        username: username.to_string(),
        password_hash: hash_password(password)?,
        created_at: Utc::now().to_rfc3339(),
    };
    let content = serde_json::to_string_pretty(&config)
        .map_err(|error| format!("cannot serialize writer authentication config: {error}"))?;
    write_private_text(path, &(content + "\n"))
}

pub fn validate_credentials(state: &AppState, username: &str, password: &str) -> Result<bool, String> {
    let config = load_auth_config(&state.auth_file)?;
    let expected_username = config.as_ref().map(|value| value.username.as_str()).unwrap_or("");
    let password_hash = config
        .as_ref()
        .map(|value| value.password_hash.as_str())
        .unwrap_or(state.dummy_password_hash.as_str());

    let password_valid = verify_password(password_hash, password);
    let username_valid = config.is_some()
        && expected_username.as_bytes().len() == username.as_bytes().len()
        && bool::from(expected_username.as_bytes().ct_eq(username.as_bytes()));

    Ok(username_valid && password_valid)
}

pub fn load_or_create_session_key(path: &Path) -> Result<String, String> {
    if let Ok(configured) = std::env::var("BLOG_SECRET_KEY") {
        let configured = configured.trim();
        if !configured.is_empty() {
            return Ok(configured.to_string());
        }
    }

    match fs::read_to_string(path) {
        Ok(value) if !value.trim().is_empty() => return Ok(value.trim().to_string()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot read session key: {error}")),
    }

    let generated = random_token(64);
    write_private_text(path, &(generated.clone() + "\n"))?;
    Ok(generated)
}

pub fn write_private_text(path: &Path, value: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }

    let temporary = path.with_extension(
        path.extension()
            .map(|ext| format!("{}.tmp", ext.to_string_lossy()))
            .unwrap_or_else(|| "tmp".to_string()),
    );
    {
        let mut file = fs::File::create(&temporary)
            .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
        file.write_all(value.as_bytes())
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync {}: {error}", temporary.display()))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

pub fn parse_session_cookie(cookie_header: Option<&str>, state: &AppState) -> SessionData {
    let Some(cookie_header) = cookie_header else {
        return SessionData::anonymous();
    };

    let encoded = cookie_header
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE).then_some(value));

    let Some(encoded) = encoded else {
        return SessionData::anonymous();
    };

    decode_session(encoded, state).unwrap_or_else(SessionData::anonymous)
}

fn decode_session(value: &str, state: &AppState) -> Option<SessionData> {
    let (payload_b64, signature_b64) = value.split_once('.')?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()).ok()?;
    let signature = URL_SAFE_NO_PAD.decode(signature_b64.as_bytes()).ok()?;

    let mut mac = HmacSha256::new_from_slice(state.session_key.as_slice()).ok()?;
    mac.update(payload_b64.as_bytes());
    mac.verify_slice(&signature).ok()?;

    let session: SessionData = serde_json::from_slice(&payload).ok()?;
    (session.expires_at > now_epoch()).then_some(session)
}

pub fn session_cookie(session: &SessionData, state: &AppState) -> Result<String, String> {
    let payload = serde_json::to_vec(session)
        .map_err(|error| format!("cannot serialize session: {error}"))?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(&payload);
    let mut mac = HmacSha256::new_from_slice(state.session_key.as_slice())
        .map_err(|error| format!("cannot sign session: {error}"))?;
    mac.update(payload_b64.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(&mac.finalize().into_bytes());

    let mut cookie = format!(
        "{SESSION_COOKIE}={payload_b64}.{signature}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL_SECONDS}"
    );
    if state.secure_cookies {
        cookie.push_str("; Secure");
    }
    Ok(cookie)
}

pub fn clear_session_cookie(state: &AppState) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
    );
    if state.secure_cookies {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn validate_totp(state: &AppState, username: &str, code: &str) -> bool {
    if !state.totp_enabled {
        return true;
    }

    let compact: String = code.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.len() != 6 || !compact.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }

    let secret = match BASE32_NOPAD.decode(state.totp_secret.to_ascii_uppercase().as_bytes()) {
        Ok(secret) if !secret.is_empty() => secret,
        _ => return false,
    };
    let supplied: u32 = match compact.parse() {
        Ok(value) => value,
        Err(_) => return false,
    };
    let current_step = now_epoch() / 30;

    for offset in -1_i64..=1 {
        let step = current_step + offset;
        if hotp(&secret, step as u64) != supplied {
            continue;
        }

        let mut replay_guard = state.last_totp_step.lock().expect("TOTP replay lock poisoned");
        if step <= replay_guard.get(username).copied().unwrap_or(-1) {
            return false;
        }
        replay_guard.insert(username.to_string(), step);
        return true;
    }

    false
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts arbitrary key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | digest[offset + 3] as u32;
    binary % 1_000_000
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trip_shape_is_serializable() {
        let session = SessionData::authenticated("dave".to_string());
        let encoded = serde_json::to_vec(&session).unwrap();
        let decoded: SessionData = serde_json::from_slice(&encoded).unwrap();
        assert!(decoded.writer_authenticated);
        assert_eq!(decoded.writer_username.as_deref(), Some("dave"));
    }

    #[test]
    fn hotp_matches_rfc_4226_vector() {
        let secret = b"12345678901234567890";
        assert_eq!(hotp(secret, 0), 755224);
        assert_eq!(hotp(secret, 1), 287082);
    }
}
