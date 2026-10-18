use serde_json::{json, Value};
use std::{net::TcpListener, time::Duration};
use tungstenite::{connect, Message};
use w2g_server::{auth::now_unix, mint, serve, Auth, TokenAuth};
use w2g_sync::Role;

const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

#[test]
fn valid_token_yields_role_and_sub() {
    let a = TokenAuth::new(SECRET);
    let t = mint(SECRET, "alice", "r1", Role::Controller, now_unix() + 60);
    let id = a.verify(&t, "r1").unwrap();
    assert_eq!(id.sub, "alice");
    assert_eq!(id.role, Role::Controller);
}

#[test]
fn wrong_room_expired_wrong_secret_and_garbage_are_rejected() {
    let a = TokenAuth::new(SECRET);
    let t = mint(SECRET, "a", "r1", Role::Watcher, now_unix() + 60);
    assert!(a.verify(&t, "r2").is_none());                                         // other room
    assert!(a.verify(&mint(SECRET, "a", "r1", Role::Watcher, now_unix() - 1), "r1").is_none()); // expired
    assert!(TokenAuth::new(b"another-secret-another-secret-xx").verify(&t, "r1").is_none());
    for g in ["", "v1", "v1.a", "v1.a.b.c", "v2.a.b", "ctl:alice", "..."] {
        assert!(a.verify(g, "r1").is_none(), "{g}");
    }
}

#[test]
fn tampering_with_the_role_breaks_the_signature() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
    let a = TokenAuth::new(SECRET);
    let t = mint(SECRET, "mallory", "r1", Role::Watcher, now_unix() + 60);
    let parts: Vec<&str> = t.split('.').collect();
    let mut p: Value = serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
    p["role"] = json!("controller");
    let forged = format!("v1.{}.{}", B64.encode(p.to_string()), parts[2]);
    assert!(a.verify(&forged, "r1").is_none());
}

#[test]
fn websocket_join_uses_the_signed_role_not_a_claimed_one() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    serve(l, TokenAuth::new(SECRET), Duration::from_millis(300));
    let url = format!("ws://{addr}");
    let tok = mint(SECRET, "bob", "r1", Role::Watcher, now_unix() + 60);
    let (mut ws, _) = connect(&url).unwrap();
    ws.send(Message::Text(json!({"t":"hello","room":"r1","token":tok,"role":"controller"}).to_string())).unwrap();
    let _first_state = ws.read().unwrap();
    ws.send(Message::Text(json!({"t":"cmd","c":"play"}).to_string())).unwrap();
    loop {
        if let Message::Text(t) = ws.read().unwrap() {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["t"] == "denied" { assert_eq!(v["why"], "role"); break; }
            assert_ne!(v["playing"], true, "watcher must not be able to start playback");
        }
    }
    // a token minted for another room cannot join this one
    let other = mint(SECRET, "bob", "r2", Role::Controller, now_unix() + 60);
    let (mut ws2, _) = connect(&url).unwrap();
    ws2.send(Message::Text(json!({"t":"hello","room":"r1","token":other}).to_string())).unwrap();
    loop { if let Message::Text(t) = ws2.read().unwrap() { assert!(t.contains("denied")); break; } }
}

/// Same literal is asserted by the Python side (tests/unit/test_sync_token.py in vergoboy-stream).
#[test]
fn python_minted_token_verifies() {
    const FIXTURE: &str = "v1.eyJleHAiOjQxMDI0NDQ4MDAsInJvbGUiOiJjb250cm9sbGVyIiwicm9vbSI6IlIxIiwic3ViIjoiYWxpY2UifQ.0v1oyyKGYHuqUWYEGS--xLVnOnyw3AfrSyCzaupyuRY";
    let a = TokenAuth::new(SECRET);
    let id = a.verify(FIXTURE, "R1").expect("token minted by the Flask side must verify");
    assert_eq!(id.sub, "alice");
    assert_eq!(id.role, Role::Controller);
    assert!(a.verify(FIXTURE, "R2").is_none());
    // and the Rust minter produces the identical bytes
    assert_eq!(mint(SECRET, "alice", "R1", Role::Controller, 4102444800), FIXTURE);
}
