//! Pure room-sync logic. The server holds the room *intent*; clients follow it.
//! No IO, no async, no dependencies, so the same crate runs on server and client.
use std::collections::VecDeque;

pub type Secs = f64;

// ---------- room state (server is the reference) ----------

#[derive(Clone, Debug, PartialEq)]
pub struct RoomState {
    pub seq: u64,
    pub playing: bool,
    pub anchor_pos: Secs,
    pub anchor_server_time: Secs,
    pub rate: f64,
    pub item_id: String,
    /// true when paused by the wait-for-buffering gate (reason `waiting`).
    pub waiting: bool,
}

impl RoomState {
    pub fn new(item_id: &str, now: Secs) -> Self {
        Self { seq: 0, playing: false, anchor_pos: 0.0, anchor_server_time: now,
               rate: 1.0, item_id: item_id.into(), waiting: false }
    }
    pub fn expected(&self, now: Secs) -> Secs {
        if self.playing {
            (self.anchor_pos + (now - self.anchor_server_time) * self.rate).max(0.0)
        } else {
            self.anchor_pos
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// play/pause carry NO position: the server computes it itself.
    Play,
    Pause,
    PauseWaiting,
    Seek { target: Secs },
    /// Optional, controllers only. `seq` is the state the client last saw.
    PauseAt { pos: Secs, seq: u64, ready: bool },
    SetRate(f64),
    SetItem { item_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role { Watcher, Controller }

#[derive(Debug, PartialEq, Eq)]
pub struct Denied;

pub const PAUSE_AT_MAX_DISTANCE: Secs = 5.0;

impl RoomState {
    /// Applies a command; every change bumps `seq`.
    pub fn apply(&self, cmd: &Cmd, now: Secs) -> RoomState {
        let exp = self.expected(now);
        let mut s = self.clone();
        s.seq += 1;
        s.anchor_server_time = now;
        s.anchor_pos = exp; // re-anchor at the room's own position
        match cmd {
            Cmd::Play => { s.playing = true; s.waiting = false; }
            Cmd::Pause => { s.playing = false; s.waiting = false; }
            Cmd::PauseWaiting => { s.playing = false; s.waiting = true; }
            Cmd::Seek { target } => s.anchor_pos = target.max(0.0),
            Cmd::PauseAt { pos, seq, ready } => {
                s.playing = false;
                s.waiting = false;
                let ok = *ready && *seq == self.seq && (pos - exp).abs() <= PAUSE_AT_MAX_DISTANCE;
                // rejected -> plain pause on expected(now), never the stale cache position
                s.anchor_pos = if ok { pos.max(0.0) } else { exp };
            }
            Cmd::SetRate(r) => s.rate = r.clamp(0.25, 4.0),
            Cmd::SetItem { item_id } => {
                s.item_id = item_id.clone();
                s.anchor_pos = 0.0;
                s.playing = false;
                s.waiting = false;
            }
        }
        s
    }

    /// Server entry point: only controllers can change the room.
    pub fn handle(&self, role: Role, cmd: &Cmd, now: Secs) -> Result<RoomState, Denied> {
        if role != Role::Controller { return Err(Denied); }
        Ok(self.apply(cmd, now))
    }
}

// ---------- clock (NTP-like, 8 samples, lowest RTT wins) ----------

#[derive(Clone, Copy, Debug)]
pub struct Sample { pub offset: Secs, pub rtt: Secs }

#[derive(Default)]
pub struct ClockSync { samples: VecDeque<Sample> }

impl ClockSync {
    /// t0 = client send, ts = server time at reply, t1 = client receive (client clock).
    pub fn push(&mut self, t0: Secs, ts: Secs, t1: Secs) {
        self.samples.push_back(Sample { offset: ts - (t0 + t1) / 2.0, rtt: t1 - t0 });
        if self.samples.len() > 8 { self.samples.pop_front(); }
    }
    pub fn best(&self) -> Option<Sample> {
        self.samples.iter().copied().min_by(|a, b| a.rtt.total_cmp(&b.rtt))
    }
    pub fn server_now(&self, local_now: Secs) -> Option<Secs> {
        self.best().map(|s| local_now + s.offset)
    }
}

// ---------- client ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Connecting, Syncing, Ready, Buffering }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Room, Local }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drift { Hold, Ok, Rate(f64), Seek(Secs) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeekKind { Echo, Seeked, UserSeek }

pub const DRIFT_IGNORE: Secs = 0.3;
pub const DRIFT_SEEK: Secs = 2.0;
pub const SEEK_COOLDOWN: Secs = 3.0;
const OWN_SEEK_TOLERANCE: Secs = 0.5;

pub struct Client {
    pub phase: Phase,
    pub mode: Mode,
    pub room: Option<RoomState>,
    last_seq: u64,
    /// guard: set while we apply a remote state, so mpv's own events are not re-sent.
    pub applying_remote: bool,
    own_seek_target: Option<Secs>,
    last_seek_at: Option<Secs>,
}

impl Client {
    pub fn new(mode: Mode) -> Self {
        Self { phase: Phase::Connecting, mode, room: None, last_seq: 0,
               applying_remote: false, own_seek_target: None, last_seek_at: None }
    }

    /// Returns false (and ignores it) for a stale or duplicate-old state.
    pub fn on_state(&mut self, st: RoomState) -> bool {
        if self.room.is_some() && st.seq < self.last_seq { return false; }
        if self.room.is_some() && st.seq == self.last_seq { return true; } // idempotent resend
        self.last_seq = st.seq;
        // A new room decision is not a drift correction: it must not wait for the seek cooldown.
        self.last_seek_at = None;
        self.room = Some(st);
        if self.phase == Phase::Connecting || self.phase == Phase::Syncing {
            self.phase = Phase::Ready;
        }
        true
    }

    /// A reconnected client starts from the server's fresh state, never from its cache.
    pub fn on_reconnect(&mut self) {
        self.phase = Phase::Syncing;
        self.room = None;
        self.last_seq = 0;
        self.own_seek_target = None;
    }
    pub fn on_connected(&mut self) { if self.phase == Phase::Connecting { self.phase = Phase::Syncing; } }

    /// Buffering is informational only: it never changes the room state.
    pub fn set_buffering(&mut self, b: bool) {
        match (self.phase, b) {
            (Phase::Ready, true) => self.phase = Phase::Buffering,
            (Phase::Buffering, false) => self.phase = Phase::Ready,
            _ => {}
        }
    }

    pub fn drift_action(&mut self, local_pos: Secs, now_server: Secs, now_local: Secs) -> Drift {
        let Some(room) = &self.room else { return Drift::Hold };
        if self.phase != Phase::Ready { return Drift::Hold; } // syncing / buffering: no correction
        let d = local_pos - room.expected(now_server);
        let cooled = self.last_seek_at.map_or(true, |t| now_local - t >= SEEK_COOLDOWN);
        let ad = d.abs();
        if !room.playing {
            if ad > DRIFT_IGNORE && cooled { return self.do_seek(room.anchor_pos, now_local); }
            return Drift::Ok;
        }
        if ad < DRIFT_IGNORE { return Drift::Ok; }
        if ad <= DRIFT_SEEK {
            let f = 0.03 + 0.02 * ((ad - DRIFT_IGNORE) / (DRIFT_SEEK - DRIFT_IGNORE));
            let r = if d > 0.0 { room.rate * (1.0 - f) } else { room.rate * (1.0 + f) };
            return Drift::Rate(r);
        }
        if cooled { let t = room.expected(now_server); return self.do_seek(t, now_local); }
        Drift::Ok
    }
    fn do_seek(&mut self, target: Secs, now_local: Secs) -> Drift {
        self.own_seek_target = Some(target);
        self.last_seek_at = Some(now_local);
        self.applying_remote = true;
        Drift::Seek(target)
    }

    /// Classifies a seek event coming back from the player.
    /// own seek -> Seeked; remote-apply echo -> Echo; anything else -> UserSeek.
    pub fn on_player_seek(&mut self, target: Secs) -> SeekKind {
        if let Some(t) = self.own_seek_target {
            if (t - target).abs() <= OWN_SEEK_TOLERANCE {
                self.own_seek_target = None;
                self.applying_remote = false;
                return SeekKind::Seeked;
            }
        }
        if self.applying_remote { return SeekKind::Echo; }
        SeekKind::UserSeek
    }

    /// The only gate through which a client may emit a command.
    pub fn emit(&self, role: Role, cmd: Cmd) -> Option<Cmd> {
        if self.mode == Mode::Local { return None; }          // local mode: no sync events
        if role != Role::Controller { return None; }         // watcher: caller reverts to room state
        if self.applying_remote { return None; }             // echo guard
        if self.phase == Phase::Connecting || self.phase == Phase::Syncing { return None; }
        Some(cmd)
    }
}

/// Collapses bursts of seeks into one (300–500 ms quiet period).
pub struct SeekDebouncer { quiet: Secs, pending: Option<(Secs, Secs)> }
impl SeekDebouncer {
    pub fn new(quiet: Secs) -> Self { Self { quiet, pending: None } }
    pub fn push(&mut self, target: Secs, now: Secs) { self.pending = Some((target, now)); }
    pub fn poll(&mut self, now: Secs) -> Option<Secs> {
        match self.pending {
            Some((t, at)) if now - at >= self.quiet => { self.pending = None; Some(t) }
            _ => None,
        }
    }
}

// ---------- optional room option: wait_for_buffering ----------

pub struct WaitGate { pub enabled: bool, pub threshold: Secs, pub timeout: Secs, paused_at: Option<Secs> }
impl WaitGate {
    pub fn new(enabled: bool, threshold: Secs, timeout: Secs) -> Self {
        Self { enabled, threshold, timeout, paused_at: None }
    }
    /// `buffering_for`: how long each currently-buffering member has been buffering.
    pub fn tick(&mut self, st: &RoomState, buffering_for: &[Secs], now: Secs) -> Option<Cmd> {
        if !self.enabled { return None; }
        let worst = buffering_for.iter().cloned().fold(0.0, f64::max);
        if st.playing && worst > self.threshold {
            self.paused_at = Some(now);
            return Some(Cmd::PauseWaiting);
        }
        if st.waiting {
            let since = self.paused_at.unwrap_or(now);
            if buffering_for.is_empty() || now - since >= self.timeout {
                self.paused_at = None;
                return Some(Cmd::Play);
            }
        }
        None
    }
}

pub mod ffi;

#[cfg(test)]
mod tests;
