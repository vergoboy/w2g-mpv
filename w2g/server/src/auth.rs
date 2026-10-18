//! Room access. A room token is minted by the account service (phase 1) and verified here:
//!   v1.<base64url(payload json)>.<base64url(HMAC-SHA256(secret, "v1."+payload))>
//! payload = {"sub","room","role":"controller"|"watcher","exp":unix_seconds}
//! The role comes ONLY from the signed token, never from what the client claims.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use w2g_sync::Role;

type H = Hmac<Sha256>;

pub struct Identity { pub sub: String, pub role: Role }

pub trait Auth: Send + Sync + 'static {
    /// Verifies `token` for joining `room`; None rejects.
    fn verify(&self, token: &str, room: &str) -> Option<Identity>;
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn mint(secret: &[u8], sub: &str, room: &str, role: Role, exp: u64) -> String {
    let role_s = match role { Role::Controller => "controller", Role::Watcher => "watcher" };
    let p = B64.encode(json!({"sub": sub, "room": room, "role": role_s, "exp": exp}).to_string());
    let mut m = H::new_from_slice(secret).expect("hmac accepts any key length");
    m.update(format!("v1.{p}").as_bytes());
    format!("v1.{p}.{}", B64.encode(m.finalize().into_bytes()))
}

pub struct TokenAuth { secret: Vec<u8> }
impl TokenAuth {
    pub fn new(secret: &[u8]) -> Self { Self { secret: secret.to_vec() } }
}
impl Auth for TokenAuth {
    fn verify(&self, token: &str, room: &str) -> Option<Identity> {
        let mut it = token.split('.');
        let (v, p, sig) = (it.next()?, it.next()?, it.next()?);
        if it.next().is_some() || v != "v1" { return None; }
        // signature first (constant-time), only then look at the payload
        let mut m = H::new_from_slice(&self.secret).ok()?;
        m.update(format!("v1.{p}").as_bytes());
        m.verify_slice(&B64.decode(sig).ok()?).ok()?;
        let payload: Value = serde_json::from_slice(&B64.decode(p).ok()?).ok()?;
        if payload.get("room")?.as_str()? != room { return None; }
        if payload.get("exp")?.as_u64()? <= now_unix() { return None; }
        let role = match payload.get("role")?.as_str()? {
            "controller" => Role::Controller, "watcher" => Role::Watcher, _ => return None };
        Some(Identity { sub: payload.get("sub")?.as_str()?.to_string(), role })
    }
}

/// DEV ONLY (W2G_DEV_AUTH=1): "ctl:<name>" is a controller, "watch:<name>" a watcher; room is ignored.
pub struct PrefixAuth;
impl Auth for PrefixAuth {
    fn verify(&self, t: &str, _room: &str) -> Option<Identity> {
        if let Some(n) = t.strip_prefix("ctl:") { Some(Identity { sub: n.into(), role: Role::Controller }) }
        else if let Some(n) = t.strip_prefix("watch:") { Some(Identity { sub: n.into(), role: Role::Watcher }) }
        else { None }
    }
}
