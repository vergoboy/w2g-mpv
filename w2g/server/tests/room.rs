use serde_json::{json, Value};
use std::{net::TcpListener, time::Duration};
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};
use w2g_server::{serve, PrefixAuth};

type Ws = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

fn start() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    serve(l, PrefixAuth, Duration::from_millis(300));
    format!("ws://{addr}")
}
fn send(ws: &mut Ws, v: Value) { ws.send(Message::Text(v.to_string())).unwrap(); }
fn recv(ws: &mut Ws) -> Value {
    loop { if let Message::Text(t) = ws.read().unwrap() { return serde_json::from_str(&t).unwrap(); } }
}
fn recv_kind(ws: &mut Ws, kind: &str) -> Value {
    loop { let v = recv(ws); if v["t"] == kind { return v; } }
}
fn join(url: &str, token: &str) -> (Ws, Value) {
    let (mut ws, _) = connect(url).unwrap();
    send(&mut ws, json!({"t":"hello","room":"r1","token":token,"item":"m1"}));
    let st = recv_kind(&mut ws, "state");
    (ws, st)
}

#[test]
fn bad_token_is_rejected() {
    let url = start();
    let (mut ws, _) = connect(&url).unwrap();
    send(&mut ws, json!({"t":"hello","token":"nope"}));
    assert_eq!(recv(&mut ws)["t"], "denied");
}

#[test]
fn clock_ping_returns_server_time_and_echoes_t0() {
    let url = start();
    let (mut ws, _) = join(&url, "watch:a");
    send(&mut ws, json!({"t":"ping","t0":123.5}));
    let p = recv_kind(&mut ws, "pong");
    assert_eq!(p["t0"], 123.5);
    assert!(p["ts"].as_f64().unwrap() >= 0.0);
}

#[test]
fn controller_play_reaches_watcher_and_watcher_cannot_move_room() {
    let url = start();
    let (mut ctl, _) = join(&url, "ctl:a");
    let (mut wat, st0) = join(&url, "watch:b");
    assert_eq!(st0["playing"], false);

    send(&mut ctl, json!({"t":"cmd","c":"play"}));
    let st = recv_kind(&mut wat, "state");
    assert_eq!(st["playing"], true);
    let seq = st["seq"].as_u64().unwrap();

    send(&mut wat, json!({"t":"cmd","c":"pause"}));
    assert_eq!(recv_kind(&mut wat, "denied")["why"], "role");
    // the periodic resend proves the room is still playing, same seq
    let again = recv_kind(&mut wat, "state");
    assert_eq!(again["playing"], true);
    assert_eq!(again["seq"].as_u64().unwrap(), seq);
}

#[test]
fn pause_at_far_from_room_falls_back_and_late_joiner_gets_fresh_state() {
    let url = start();
    let (mut ctl, _) = join(&url, "ctl:a");
    send(&mut ctl, json!({"t":"cmd","c":"seek","target":3630.0}));
    let st = recv_kind(&mut ctl, "state");
    send(&mut ctl, json!({"t":"cmd","c":"play"}));
    let st = { let _ = st; recv_kind(&mut ctl, "state") };
    let seq = st["seq"].as_u64().unwrap();
    // stale cache position 70 s while the room is at ~3630 s -> pause lands near 3630, not 70
    send(&mut ctl, json!({"t":"cmd","c":"pause_at","pos":70.0,"seq":seq,"ready":true}));
    let p = recv_kind(&mut ctl, "state");
    assert_eq!(p["playing"], false);
    assert!(p["anchor_pos"].as_f64().unwrap() > 3600.0, "{p}");
    // reconnecting user starts from the server's state
    let (_late, fresh) = join(&url, "watch:late");
    assert_eq!(fresh["seq"], p["seq"]);
    assert_eq!(fresh["anchor_pos"], p["anchor_pos"]);
}
