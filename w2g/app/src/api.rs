//! Client for the existing Flask API (`/api/auth/*`, `/api/room/*`, `/api/state`).
use crate::session::{self, Session};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum ApiError {
    /// no usable session: log in again
    LoginRequired,
    /// server answered with an error; message is the server's own (Persian) `error` text
    Http(u16, String),
    Net(String),
    Io(String),
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::LoginRequired => write!(f, "not logged in (run `w2g login`)"),
            ApiError::Http(c, m) if m.is_empty() => write!(f, "server error {c}"),
            ApiError::Http(c, m) => write!(f, "{m} ({c})"),
            ApiError::Net(m) => write!(f, "network error: {m}"),
            ApiError::Io(m) => write!(f, "disk error: {m}"),
        }
    }
}
impl From<std::io::Error> for ApiError { fn from(e: std::io::Error) -> Self { ApiError::Io(e.to_string()) } }

pub struct Api {
    agent: ureq::Agent,
    dir: PathBuf,
    base: String,
    session: Option<Session>,
}

fn field(v: &Value, k: &str) -> Result<String, ApiError> {
    v.get(k).and_then(Value::as_str).map(String::from)
        .ok_or_else(|| ApiError::Net(format!("unexpected response: missing `{k}`")))
}

impl Api {
    /// `base` is the API root, e.g. `https://vergoboy.ir/w2g`. Loads a saved session if present.
    pub fn open(dir: impl Into<PathBuf>, base: &str) -> Result<Api, ApiError> {
        let dir = dir.into();
        let session = session::load(&dir)?.filter(|s| s.api == base.trim_end_matches('/'));
        Ok(Api {
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build(),
            dir, base: base.trim_end_matches('/').to_string(), session,
        })
    }
    pub fn session(&self) -> Option<&Session> { self.session.as_ref() }
    pub fn base(&self) -> &str { &self.base }

    fn request(&self, method: &str, path: &str, body: Option<&Value>, bearer: Option<&str>) -> Result<Value, ApiError> {
        let mut req = self.agent.request(method, &format!("{}{}", self.base, path));
        if let Some(t) = bearer { req = req.set("Authorization", &format!("Bearer {t}")); }
        let res = match body { Some(b) => req.send_json(b.clone()), None => req.call() };
        match res {
            Ok(r) => r.into_json::<Value>().map_err(|e| ApiError::Net(e.to_string())),
            Err(ureq::Error::Status(code, r)) => {
                let msg = r.into_json::<Value>().ok()
                    .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from)).unwrap_or_default();
                Err(ApiError::Http(code, msg))
            }
            Err(e) => Err(ApiError::Net(e.to_string())),
        }
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<&Session, ApiError> {
        let v = self.request("POST", "/api/auth/login", Some(&json!({"username": username, "password": password})), None)?;
        let user = v.get("user").cloned().unwrap_or(Value::Null);
        let s = Session {
            api: self.base.clone(),
            access_token: field(&v, "access_token")?,
            refresh_token: field(&v, "refresh_token")?,
            user_id: field(&user, "id")?,
            username: field(&user, "username")?,
        };
        session::save(&self.dir, &s)?;
        self.session = Some(s);
        Ok(self.session.as_ref().unwrap())
    }

    pub fn logout(&mut self) {
        if let Some(s) = &self.session {
            let _ = self.request("POST", "/api/auth/logout", Some(&json!({"refresh_token": s.refresh_token})), Some(&s.access_token));
        }
        session::clear(&self.dir);
        self.session = None;
    }

    fn refresh(&mut self) -> Result<(), ApiError> {
        let cur = self.session.clone().ok_or(ApiError::LoginRequired)?;
        // Another w2g process may have rotated the token already; the old one is dead now.
        if let Some(disk) = session::load(&self.dir)? {
            if disk.refresh_token != cur.refresh_token { self.session = Some(disk); return Ok(()); }
        }
        match self.request("POST", "/api/auth/refresh", Some(&json!({"refresh_token": cur.refresh_token})), None) {
            Ok(v) => {
                let mut s = cur;
                s.access_token = field(&v, "access_token")?;
                s.refresh_token = field(&v, "refresh_token")?;
                session::save(&self.dir, &s)?;           // persist BEFORE using the rotated token
                self.session = Some(s);
                Ok(())
            }
            Err(ApiError::Http(401 | 403, _)) => { session::clear(&self.dir); self.session = None; Err(ApiError::LoginRequired) }
            Err(e) => Err(e),
        }
    }

    fn call(&mut self, method: &str, path: &str, body: Option<Value>) -> Result<Value, ApiError> {
        let tok = self.session.as_ref().ok_or(ApiError::LoginRequired)?.access_token.clone();
        match self.request(method, path, body.as_ref(), Some(&tok)) {
            Err(ApiError::Http(401, _)) => {
                self.refresh()?;
                let tok = self.session.as_ref().ok_or(ApiError::LoginRequired)?.access_token.clone();
                match self.request(method, path, body.as_ref(), Some(&tok)) {
                    Err(ApiError::Http(401, _)) => Err(ApiError::LoginRequired),
                    other => other,
                }
            }
            other => other,
        }
    }

    pub fn me(&mut self) -> Result<Value, ApiError> { self.call("GET", "/api/auth/me", None) }
    pub fn join_room(&mut self, code: &str) -> Result<Value, ApiError> {
        self.call("POST", "/api/room/join", Some(json!({"code": code})))
    }
    pub fn state(&mut self) -> Result<Value, ApiError> { self.call("GET", "/api/state", None) }
    pub fn sync_token(&mut self) -> Result<SyncInfo, ApiError> {
        let v = self.call("POST", "/api/room/sync-token", Some(json!({})))?;
        Ok(SyncInfo { url: field(&v, "url")?, token: field(&v, "token")?, room: field(&v, "room")?, role: field(&v, "role")? })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncInfo { pub url: String, pub token: String, pub room: String, pub role: String }
