//! C ABI for the player plugin. Header: ../include/w2g_sync.h (keep in lockstep).
//! All functions are null-safe; the handle is owned by the caller until `w2g_client_free`.
use crate::*;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct W2gState {
    pub seq: u64,
    pub playing: i32,
    pub waiting: i32,
    pub anchor_pos: f64,
    pub anchor_server_time: f64,
    pub rate: f64,
}

#[repr(C)]
pub struct W2gDrift {
    /// 0 hold, 1 ok, 2 rate (value = speed), 3 seek (value = target seconds)
    pub kind: i32,
    pub value: f64,
}

pub struct W2gClient(Client);

fn role(r: i32) -> Role { if r == 1 { Role::Controller } else { Role::Watcher } }

#[no_mangle]
pub extern "C" fn w2g_client_new(local_mode: i32) -> *mut W2gClient {
    let mode = if local_mode != 0 { Mode::Local } else { Mode::Room };
    Box::into_raw(Box::new(W2gClient(Client::new(mode))))
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_free(c: *mut W2gClient) {
    if !c.is_null() { drop(Box::from_raw(c)); }
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_on_connected(c: *mut W2gClient) {
    if let Some(c) = c.as_mut() { c.0.on_connected(); }
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_on_reconnect(c: *mut W2gClient) {
    if let Some(c) = c.as_mut() { c.0.on_reconnect(); }
}
/// returns 1 if accepted, 0 if stale/null
#[no_mangle]
pub unsafe extern "C" fn w2g_client_on_state(c: *mut W2gClient, s: *const W2gState) -> i32 {
    let (Some(c), Some(s)) = (c.as_mut(), s.as_ref()) else { return 0 };
    let st = RoomState {
        seq: s.seq, playing: s.playing != 0, anchor_pos: s.anchor_pos,
        anchor_server_time: s.anchor_server_time, rate: s.rate,
        item_id: String::new(), waiting: s.waiting != 0,
    };
    c.0.on_state(st) as i32
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_set_buffering(c: *mut W2gClient, b: i32) {
    if let Some(c) = c.as_mut() { c.0.set_buffering(b != 0); }
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_drift(
    c: *mut W2gClient, local_pos: f64, now_server: f64, now_local: f64, out: *mut W2gDrift,
) -> i32 {
    let (Some(c), Some(out)) = (c.as_mut(), out.as_mut()) else { return 0 };
    let (k, v) = match c.0.drift_action(local_pos, now_server, now_local) {
        Drift::Hold => (0, 0.0), Drift::Ok => (1, 0.0),
        Drift::Rate(r) => (2, r), Drift::Seek(t) => (3, t),
    };
    out.kind = k; out.value = v;
    1
}
/// 0 echo, 1 own seek confirmed, 2 user seek
#[no_mangle]
pub unsafe extern "C" fn w2g_client_player_seek(c: *mut W2gClient, target: f64) -> i32 {
    let Some(c) = c.as_mut() else { return 2 };
    match c.0.on_player_seek(target) { SeekKind::Echo => 0, SeekKind::Seeked => 1, SeekKind::UserSeek => 2 }
}
/// 1 if this client may emit a command now (room mode, controller, not applying remote, synced)
#[no_mangle]
pub unsafe extern "C" fn w2g_client_can_emit(c: *mut W2gClient, role_controller: i32) -> i32 {
    let Some(c) = c.as_ref() else { return 0 };
    c.0.emit(role(role_controller), Cmd::Pause).is_some() as i32
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_expected(c: *mut W2gClient, now_server: f64, out_playing: *mut i32) -> f64 {
    let Some(c) = c.as_ref() else { return -1.0 };
    match &c.0.room {
        Some(r) => { if let Some(p) = out_playing.as_mut() { *p = r.playing as i32; } r.expected(now_server) }
        None => -1.0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn w2g_client_room_rate(c: *mut W2gClient) -> f64 {
    c.as_ref().and_then(|c| c.0.room.as_ref()).map_or(1.0, |r| r.rate)
}
