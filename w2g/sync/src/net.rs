//! Player-side transport (feature `net`): one background thread owns the WebSocket,
//! reconnects with backoff, keeps an NTP-like clock, and hands states to the plugin by polling.
use crate::{ffi::W2gState, ClockSync};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::ffi::CStr;
use std::io::ErrorKind;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::{connect, stream::MaybeTlsStream, Error as WsError, Message};

struct Shared {
    states: Mutex<VecDeque<W2gState>>,
    reconnected: AtomicBool,
    clock: Mutex<ClockSync>,
    stop: AtomicBool,
    start: Instant,
}
impl Shared { fn local_now(&self) -> f64 { self.start.elapsed().as_secs_f64() } }

pub struct W2gNet { sh: Arc<Shared>, tx: Sender<String>, th: Option<JoinHandle<()>> }

fn session(sh: &Shared, rx: &Receiver<String>, url: &str, token: &str, room: &str) -> Option<()> {
    let (mut ws, _) = connect(url).ok()?;
    if let MaybeTlsStream::Plain(s) = ws.get_mut() { s.set_read_timeout(Some(Duration::from_millis(100))).ok()?; }
    ws.send(Message::Text(json!({"t":"hello","room":room,"token":token}).to_string())).ok()?;
    let (mut last_ping, mut pongs): (Option<Instant>, u32) = (None, 0);
    loop {
        if sh.stop.load(Ordering::Relaxed) { return Some(()); }
        let every = if pongs < 8 { Duration::from_millis(200) } else { Duration::from_secs(10) };
        if last_ping.map_or(true, |t| t.elapsed() >= every) {
            ws.send(Message::Text(json!({"t":"ping","t0":sh.local_now()}).to_string())).ok()?;
            last_ping = Some(Instant::now());
        }
        while let Ok(m) = rx.try_recv() { ws.send(Message::Text(m)).ok()?; }
        match ws.read() {
            Ok(Message::Text(t)) => {
                let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                match v.get("t").and_then(Value::as_str) {
                    Some("state") => {
                        let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false) as i32;
                        sh.states.lock().unwrap().push_back(W2gState {
                            seq: v.get("seq").and_then(Value::as_u64).unwrap_or(0),
                            playing: b("playing"), waiting: b("waiting"),
                            anchor_pos: f("anchor_pos"), anchor_server_time: f("anchor_server_time"), rate: f("rate"),
                        });
                    }
                    Some("pong") => {
                        let (Some(t0), Some(ts)) = (v.get("t0").and_then(Value::as_f64), v.get("ts").and_then(Value::as_f64)) else { continue };
                        sh.clock.lock().unwrap().push(t0, ts, sh.local_now());
                        pongs += 1;
                    }
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => return None,
            Ok(_) => {}
            Err(WsError::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return None,
        }
    }
}

fn run(sh: Arc<Shared>, rx: Receiver<String>, url: String, token: String, room: String) {
    let mut first = true;
    while !sh.stop.load(Ordering::Relaxed) {
        if !first { sh.reconnected.store(true, Ordering::Relaxed); }
        first = false;
        let _ = session(&sh, &rx, &url, &token, &room);
        for _ in 0..10 { if sh.stop.load(Ordering::Relaxed) { return; } std::thread::sleep(Duration::from_millis(100)); }
    }
}

unsafe fn s(p: *const c_char) -> String { if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() } }

#[no_mangle]
pub unsafe extern "C" fn w2g_net_start(url: *const c_char, token: *const c_char, room: *const c_char) -> *mut W2gNet {
    let sh = Arc::new(Shared { states: Mutex::new(VecDeque::new()), reconnected: AtomicBool::new(false),
        clock: Mutex::new(ClockSync::default()), stop: AtomicBool::new(false), start: Instant::now() });
    let (tx, rx) = channel();
    let (sh2, u, t, r) = (sh.clone(), s(url), s(token), s(room));
    let th = std::thread::spawn(move || run(sh2, rx, u, t, r));
    Box::into_raw(Box::new(W2gNet { sh, tx, th: Some(th) }))
}
#[no_mangle]
pub unsafe extern "C" fn w2g_net_free(n: *mut W2gNet) {
    if n.is_null() { return; }
    let mut n = Box::from_raw(n);
    n.sh.stop.store(true, Ordering::Relaxed);
    if let Some(t) = n.th.take() { let _ = t.join(); }
}
/// pops one queued state; 1 if `out` was filled
#[no_mangle]
pub unsafe extern "C" fn w2g_net_poll_state(n: *mut W2gNet, out: *mut W2gState) -> i32 {
    let (Some(n), Some(out)) = (n.as_ref(), out.as_mut()) else { return 0 };
    match n.sh.states.lock().unwrap().pop_front() { Some(st) => { *out = st; 1 } None => 0 }
}
/// 1 once after every reconnect (clears the flag)
#[no_mangle]
pub unsafe extern "C" fn w2g_net_poll_reconnect(n: *mut W2gNet) -> i32 {
    n.as_ref().map_or(0, |n| n.sh.reconnected.swap(false, Ordering::Relaxed) as i32)
}
/// server time in seconds, or -1.0 until the clock has samples
#[no_mangle]
pub unsafe extern "C" fn w2g_net_server_now(n: *mut W2gNet) -> f64 {
    let Some(n) = n.as_ref() else { return -1.0 };
    n.sh.clock.lock().unwrap().server_now(n.sh.local_now()).unwrap_or(-1.0)
}
/// cmd: 0 play, 1 pause, 2 seek(a=target), 3 pause_at(a=pos, seq, ready), 4 rate(a)
#[no_mangle]
pub unsafe extern "C" fn w2g_net_send(n: *mut W2gNet, cmd: i32, a: f64, seq: u64, ready: i32) {
    let Some(n) = n.as_ref() else { return };
    let m = match cmd {
        0 => json!({"t":"cmd","c":"play"}),
        1 => json!({"t":"cmd","c":"pause"}),
        2 => json!({"t":"cmd","c":"seek","target":a}),
        3 => json!({"t":"cmd","c":"pause_at","pos":a,"seq":seq,"ready":ready != 0}),
        4 => json!({"t":"cmd","c":"rate","rate":a}),
        _ => return,
    };
    let _ = n.tx.send(m.to_string());
}
