//! The client against a mock that mimics the real Flask API (shapes read from app.py).
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use w2g_app::api::{Api, ApiError};
use w2g_app::session::{self, Session};

type Handler = Arc<dyn Fn(&str, &str, &str, &Value) -> (u16, Value) + Send + Sync>;

/// returns base URL like http://127.0.0.1:PORT/w2g ; handler(method, path, bearer, body)
fn mock(h: Handler) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let h = h.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut it = line.split_whitespace();
                let (method, path) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string());
                let (mut len, mut bearer) = (0usize, String::new());
                loop {
                    let mut hl = String::new();
                    r.read_line(&mut hl).unwrap();
                    if hl.trim().is_empty() { break; }
                    let low = hl.to_ascii_lowercase();
                    if let Some(v) = low.strip_prefix("content-length:") { len = v.trim().parse().unwrap_or(0); }
                    if low.starts_with("authorization:") { bearer = hl.split_whitespace().nth(2).unwrap_or("").to_string(); }
                }
                let mut body = vec![0u8; len];
                r.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let (code, out) = h(&method, path.trim_start_matches("/w2g"), &bearer, &body);
                let out = out.to_string();
                let mut s = s;
                write!(s, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).unwrap();
            });
        }
    });
    format!("http://127.0.0.1:{port}/w2g")
}

fn tmp() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("w2g-app-test-{}-{}", std::process::id(), rand_id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
fn rand_id() -> u128 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() }

fn login_ok() -> Value {
    // same shape as api_auth_login: tokens + user + room
    json!({"access_token":"acc1","refresh_token":"ref1",
           "user":{"id":"u1","username":"alice","email_verified":true},"room":{"id":"R1","name":"Alice"}})
}

#[test]
fn login_is_saved_to_disk_and_survives_a_restart() {
    let base = mock(Arc::new(|m, p, _, b| match (m, p) {
        ("POST", "/api/auth/login") if b["username"] == "alice" && b["password"] == "pw" => (200, login_ok()),
        ("POST", "/api/auth/login") => (401, json!({"error":"نام کاربری یا رمز عبور اشتباه است"})),
        ("GET", "/api/auth/me") => (200, json!({"user":{"username":"alice"}})),
        _ => (404, json!({})),
    }));
    let dir = tmp();
    assert_eq!(Api::open(&dir, &base).unwrap().login("alice", "bad").unwrap_err(),
               ApiError::Http(401, "نام کاربری یا رمز عبور اشتباه است".into()));
    assert!(session::load(&dir).unwrap().is_none(), "a failed login must not leave a session");

    Api::open(&dir, &base).unwrap().login("alice", "pw").unwrap();
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(dir.join("session.json")).unwrap().permissions().mode() & 0o777;
      assert_eq!(mode, 0o600, "tokens must not be world-readable"); }

    // "next launch": a brand-new Api object, no password anywhere
    let mut again = Api::open(&dir, &base).unwrap();
    assert_eq!(again.session().unwrap().username, "alice");
    assert_eq!(again.me().unwrap()["user"]["username"], "alice");
}

#[test]
fn expired_access_token_is_refreshed_and_the_rotated_token_is_persisted() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let c2 = calls.clone();
    let base = mock(Arc::new(move |m, p, bearer, b| {
        c2.lock().unwrap().push(format!("{m} {p}"));
        match (m, p) {
            ("POST", "/api/auth/refresh") if b["refresh_token"] == "ref1" =>
                (200, json!({"access_token":"acc2","refresh_token":"ref2","user":{"id":"u1"}})),
            ("POST", "/api/auth/refresh") => (401, json!({"error":"نشست نامعتبر است؛ دوباره وارد شو"})),
            ("GET", "/api/state") if bearer == "acc2" => (200, json!({"playlist":[],"current_index":0})),
            ("GET", "/api/state") => (401, json!({"error":"expired"})),
            _ => (404, json!({})),
        }
    }));
    let dir = tmp();
    session::save(&dir, &Session { api: base.clone(), access_token: "acc1".into(), refresh_token: "ref1".into(),
                                   user_id: "u1".into(), username: "alice".into() }).unwrap();
    let mut api = Api::open(&dir, &base).unwrap();
    assert_eq!(api.state().unwrap()["current_index"], 0);
    let on_disk = session::load(&dir).unwrap().unwrap();
    assert_eq!((on_disk.access_token.as_str(), on_disk.refresh_token.as_str()), ("acc2", "ref2"));
    assert_eq!(calls.lock().unwrap().iter().filter(|c| c.contains("refresh")).count(), 1);
    // restart: it keeps working with the rotated token, no further refresh needed
    let mut next = Api::open(&dir, &base).unwrap();
    assert!(next.state().is_ok());
    assert_eq!(calls.lock().unwrap().iter().filter(|c| c.contains("refresh")).count(), 1);
}

#[test]
fn dead_refresh_token_means_login_required_and_the_session_is_cleared() {
    let base = mock(Arc::new(|m, p, _, _| match (m, p) {
        ("POST", "/api/auth/refresh") => (401, json!({"error":"نشست منقضی شده؛ دوباره وارد شو"})),
        _ => (401, json!({"error":"x"})),
    }));
    let dir = tmp();
    session::save(&dir, &Session { api: base.clone(), access_token: "a".into(), refresh_token: "r".into(),
                                   user_id: "u".into(), username: "alice".into() }).unwrap();
    let mut api = Api::open(&dir, &base).unwrap();
    assert_eq!(api.state().unwrap_err(), ApiError::LoginRequired);
    assert!(session::load(&dir).unwrap().is_none());
    assert_eq!(api.state().unwrap_err(), ApiError::LoginRequired);
}

#[test]
fn a_transient_network_failure_does_not_log_the_user_out() {
    let dir = tmp();
    // nothing listens on this port
    let dead = "http://127.0.0.1:9/w2g".to_string();
    session::save(&dir, &Session { api: dead.clone(), access_token: "a".into(), refresh_token: "r".into(),
                                   user_id: "u".into(), username: "alice".into() }).unwrap();
    let mut api = Api::open(&dir, &dead).unwrap();
    assert!(matches!(api.state().unwrap_err(), ApiError::Net(_)));
    assert!(session::load(&dir).unwrap().is_some(), "offline must keep the session");
}

#[test]
fn a_token_rotated_by_another_process_is_adopted_not_refreshed_again() {
    let refreshes = Arc::new(Mutex::new(0));
    let r2 = refreshes.clone();
    let base = mock(Arc::new(move |m, p, bearer, _| match (m, p) {
        ("POST", "/api/auth/refresh") => { *r2.lock().unwrap() += 1; (401, json!({"error":"old token is dead"})) }
        ("GET", "/api/state") if bearer == "acc-other" => (200, json!({"ok":true})),
        _ => (401, json!({})),
    }));
    let dir = tmp();
    let mk = |a: &str, r: &str| Session { api: base.clone(), access_token: a.into(), refresh_token: r.into(), user_id: "u".into(), username: "alice".into() };
    session::save(&dir, &mk("acc-old", "ref-old")).unwrap();
    let mut api = Api::open(&dir, &base).unwrap();
    session::save(&dir, &mk("acc-other", "ref-other")).unwrap();   // a second w2g process rotated meanwhile
    assert_eq!(api.state().unwrap()["ok"], true);
    assert_eq!(*refreshes.lock().unwrap(), 0, "must not spend the dead refresh token");
}

#[test]
fn server_error_text_is_surfaced_and_sync_token_is_parsed() {
    let base = mock(Arc::new(|m, p, _, b| match (m, p) {
        ("POST", "/api/auth/login") => (200, login_ok()),
        ("POST", "/api/room/join") if b["code"] == "NOPE" => (404, json!({"error":"اتاق با کد NOPE پیدا نشد"})),
        ("POST", "/api/room/sync-token") =>
            (200, json!({"url":"wss://vergoboy.ir/sync","token":"v1.p.s","room":"R1","role":"controller","expires_at":4102444800u64})),
        _ => (404, json!({})),
    }));
    let dir = tmp();
    let mut api = Api::open(&dir, &base).unwrap();
    api.login("alice", "pw").unwrap();
    assert_eq!(api.join_room("NOPE").unwrap_err(), ApiError::Http(404, "اتاق با کد NOPE پیدا نشد".into()));
    let s = api.sync_token().unwrap();
    assert_eq!((s.url.as_str(), s.room.as_str(), s.role.as_str()), ("wss://vergoboy.ir/sync", "R1", "controller"));
}

#[test]
fn a_session_for_another_server_is_ignored() {
    let dir = tmp();
    session::save(&dir, &Session { api: "https://other.example/w2g".into(), access_token: "a".into(),
                                   refresh_token: "r".into(), user_id: "u".into(), username: "x".into() }).unwrap();
    assert!(Api::open(&dir, "https://vergoboy.ir/w2g").unwrap().session().is_none());
}
