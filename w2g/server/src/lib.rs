//! Room sync server. Holds room *intent* (w2g_sync::RoomState); clients follow it.
//! Std threads + tungstenite: small, no async runtime.
//! AUTH IS A PLACEHOLDER: roles come from `Auth`. Replace with real tokens in phase 1.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{accept, Error as WsError, Message};
use w2g_sync::{Cmd, Role, RoomState};

pub trait Auth: Send + Sync + 'static {
    /// token -> role, or None to reject.
    fn role_for(&self, token: &str) -> Option<Role>;
}
/// Test/dev only: "ctl:*" is a controller, "watch:*" a watcher.
pub struct PrefixAuth;
impl Auth for PrefixAuth {
    fn role_for(&self, t: &str) -> Option<Role> {
        if t.starts_with("ctl:") { Some(Role::Controller) }
        else if t.starts_with("watch:") { Some(Role::Watcher) }
        else { None }
    }
}

struct Room { state: RoomState, clients: Vec<(u64, Sender<String>)> }
struct Shared { rooms: HashMap<String, Room>, next_id: u64 }

pub struct Server { start: Instant, shared: Mutex<Shared>, auth: Box<dyn Auth> }

fn state_json(s: &RoomState) -> String {
    json!({"t":"state","seq":s.seq,"playing":s.playing,"waiting":s.waiting,
           "anchor_pos":s.anchor_pos,"anchor_server_time":s.anchor_server_time,
           "rate":s.rate,"item":s.item_id}).to_string()
}

impl Server {
    pub fn now(&self) -> f64 { self.start.elapsed().as_secs_f64() }

    fn broadcast(&self, room: &Room) {
        let m = state_json(&room.state);
        for (_, tx) in &room.clients { let _ = tx.send(m.clone()); }
    }

    fn parse_cmd(v: &Value) -> Option<Cmd> {
        let f = |k: &str| v.get(k).and_then(Value::as_f64);
        Some(match v.get("c")?.as_str()? {
            "play" => Cmd::Play,
            "pause" => Cmd::Pause,
            "seek" => Cmd::Seek { target: f("target")? },
            "pause_at" => Cmd::PauseAt { pos: f("pos")?, seq: v.get("seq")?.as_u64()?,
                                         ready: v.get("ready")?.as_bool()? },
            "rate" => Cmd::SetRate(f("rate")?),
            "item" => Cmd::SetItem { item_id: v.get("item")?.as_str()?.to_string() },
            _ => return None,
        })
    }

    fn handle_conn(self: &Arc<Self>, stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        let Ok(mut ws) = accept(stream) else { return };
        let (tx, rx): (Sender<String>, Receiver<String>) = channel();
        let mut joined: Option<(String, u64, Role)> = None;
        loop {
            while let Ok(m) = rx.try_recv() { if ws.send(Message::Text(m)).is_err() { return self.leave(&joined); } }
            let msg = match ws.read() {
                Ok(m) => m,
                Err(WsError::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
                Err(_) => return self.leave(&joined),
            };
            let Message::Text(txt) = msg else { if msg.is_close() { return self.leave(&joined); } continue };
            let Ok(v) = serde_json::from_str::<Value>(&txt) else { continue };
            match v.get("t").and_then(Value::as_str) {
                Some("ping") => { // clock sync: client keeps t0, we stamp server time
                    let _ = ws.send(Message::Text(json!({"t":"pong","t0":v.get("t0"),"ts":self.now()}).to_string()));
                }
                Some("hello") if joined.is_none() => {
                    let token = v.get("token").and_then(Value::as_str).unwrap_or("");
                    let Some(role) = self.auth.role_for(token) else {
                        let _ = ws.send(Message::Text(json!({"t":"denied","why":"auth"}).to_string()));
                        return;
                    };
                    let room_id = v.get("room").and_then(Value::as_str).unwrap_or("default").to_string();
                    let item = v.get("item").and_then(Value::as_str).unwrap_or("").to_string();
                    let now = self.now();
                    let mut g = self.shared.lock().unwrap();
                    let id = g.next_id; g.next_id += 1;
                    let room = g.rooms.entry(room_id.clone())
                        .or_insert_with(|| Room { state: RoomState::new(&item, now), clients: vec![] });
                    room.clients.push((id, tx.clone()));
                    // a (re)joining client always starts from the server's fresh state
                    let _ = ws.send(Message::Text(state_json(&room.state)));
                    joined = Some((room_id, id, role));
                }
                Some("cmd") => {
                    let Some((room_id, _, role)) = &joined else { continue };
                    let Some(cmd) = Self::parse_cmd(&v) else { continue };
                    let now = self.now();
                    let mut g = self.shared.lock().unwrap();
                    let Some(room) = g.rooms.get_mut(room_id) else { continue };
                    match room.state.handle(*role, &cmd, now) {
                        Ok(s) => { room.state = s; self.broadcast(room); }
                        Err(_) => { let _ = ws.send(Message::Text(json!({"t":"denied","why":"role"}).to_string())); }
                    }
                }
                _ => {}
            }
        }
    }

    fn leave(&self, joined: &Option<(String, u64, Role)>) {
        if let Some((room, id, _)) = joined {
            let mut g = self.shared.lock().unwrap();
            if let Some(r) = g.rooms.get_mut(room) { r.clients.retain(|(i, _)| i != id); }
        }
    }
}

/// Starts accepting connections on `listener`; also re-sends full state every `resend` (idempotent).
pub fn serve(listener: TcpListener, auth: impl Auth, resend: Duration) -> Arc<Server> {
    let srv = Arc::new(Server { start: Instant::now(),
        shared: Mutex::new(Shared { rooms: HashMap::new(), next_id: 1 }), auth: Box::new(auth) });
    let s2 = srv.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(resend);
        let g = s2.shared.lock().unwrap();
        for r in g.rooms.values() { s2.broadcast(r); }
    });
    let s3 = srv.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let s = s3.clone();
            std::thread::spawn(move || s.handle_conn(conn));
        }
    });
    srv
}
