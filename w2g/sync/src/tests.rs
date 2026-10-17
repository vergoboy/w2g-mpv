use super::*;

fn playing_room(pos: Secs, at: Secs) -> RoomState {
    let s = RoomState::new("m1", 0.0);
    let s = s.apply(&Cmd::Seek { target: pos }, at);
    s.apply(&Cmd::Play, at)
}
fn ready_client(room: RoomState) -> Client {
    let mut c = Client::new(Mode::Room);
    c.on_connected();
    assert!(c.on_state(room));
    c
}

#[test]
fn lagging_user_pauses_and_room_returns() {
    // room is at 1:12, a lagging user sits on 1:10 and pauses there.
    let room = playing_room(72.0, 100.0);
    let s = room.handle(Role::Controller,
        &Cmd::PauseAt { pos: 70.0, seq: room.seq, ready: true }, 100.0).unwrap();
    assert!(!s.playing);
    assert_eq!(s.expected(500.0), 70.0); // everybody returns to 1:10
}

#[test]
fn pause_at_rejected_when_far() {
    // stale cache 1:10 while the room is at 60:30 -> pause at 60:30, not at the start
    let room = playing_room(3630.0, 100.0);
    let s = room.apply(&Cmd::PauseAt { pos: 70.0, seq: room.seq, ready: true }, 100.0);
    assert_eq!(s.anchor_pos, 3630.0);
    // not ready, or stale seq, also degrades to a plain pause on expected(now)
    let s2 = room.apply(&Cmd::PauseAt { pos: 3631.0, seq: room.seq, ready: false }, 102.0);
    assert_eq!(s2.anchor_pos, 3632.0);
    let s3 = room.apply(&Cmd::PauseAt { pos: 3631.0, seq: room.seq + 9, ready: true }, 102.0);
    assert_eq!(s3.anchor_pos, 3632.0);
}

#[test]
fn play_pause_carry_no_position_and_bump_seq() {
    let r = RoomState::new("m1", 0.0);
    let p = r.apply(&Cmd::Play, 10.0);
    assert_eq!(p.seq, r.seq + 1);
    let q = p.apply(&Cmd::Pause, 14.0);
    assert_eq!(q.anchor_pos, 4.0); // server computed it itself
}

#[test]
fn stale_seq_dropped() {
    let a = playing_room(10.0, 0.0);
    let b = a.apply(&Cmd::Pause, 1.0);
    let mut c = ready_client(b.clone());
    assert!(!c.on_state(a));                 // older seq discarded
    assert_eq!(c.room.as_ref().unwrap().seq, b.seq);
    assert!(c.on_state(b));                  // periodic resend is idempotent
}

#[test]
fn echo_does_not_loop() {
    let room = playing_room(100.0, 0.0);
    let mut c = ready_client(room);
    // we drift by 5 s -> we seek ourselves
    let d = c.drift_action(95.0, 0.0, 0.0);
    let Drift::Seek(t) = d else { panic!("expected seek, got {d:?}") };
    assert!(c.emit(Role::Controller, Cmd::Pause).is_none()); // guard is up while applying
    assert_eq!(c.on_player_seek(t), SeekKind::Seeked);       // own seek, matched
    assert!(c.emit(Role::Controller, Cmd::Pause).is_some());
    // a mismatched seek is a real user seek
    assert_eq!(c.on_player_seek(10.0), SeekKind::UserSeek);
}

#[test]
fn reconnect_resumes_at_room_position() {
    let room = playing_room(50.0, 0.0);
    let mut c = ready_client(room.clone());
    c.on_reconnect();
    assert_eq!(c.phase, Phase::Syncing);
    assert!(c.emit(Role::Controller, Cmd::Pause).is_none()); // nothing sent while syncing
    assert_eq!(c.drift_action(0.0, 0.0, 0.0), Drift::Hold);  // time-pos is not truth yet
    let later = room.apply(&Cmd::Seek { target: 3600.0 }, 20.0);
    assert!(c.on_state(later.clone()));                       // fresh server state, not cache
    assert_eq!(c.room.as_ref().unwrap().expected(20.0), 3600.0);
    assert_eq!(c.phase, Phase::Ready);
}

#[test]
fn drift_speed_vs_seek_thresholds() {
    let room = playing_room(100.0, 0.0);
    let mut c = ready_client(room);
    assert_eq!(c.drift_action(100.2, 0.0, 0.0), Drift::Ok);             // < 0.3 s
    match c.drift_action(100.5, 0.0, 0.0) {                              // 0.3–2 s -> speed
        Drift::Rate(r) => assert!(r < 1.0 && r >= 0.95 && r <= 0.97, "{r}"),
        d => panic!("{d:?}"),
    }
    match c.drift_action(99.0, 0.0, 0.0) {                               // behind -> faster
        Drift::Rate(r) => assert!(r > 1.0 && r <= 1.05),
        d => panic!("{d:?}"),
    }
    assert!(matches!(c.drift_action(110.0, 0.0, 10.0), Drift::Seek(_))); // > 2 s -> seek
    c.on_player_seek(100.0);
    assert_eq!(c.drift_action(110.0, 0.0, 11.0), Drift::Ok);             // cooldown 3 s
    assert!(matches!(c.drift_action(110.0, 0.0, 13.5), Drift::Seek(_)));
    c.set_buffering(true);
    assert_eq!(c.drift_action(110.0, 0.0, 30.0), Drift::Hold);           // no fixing while buffering
}

#[test]
fn buffering_never_changes_room_state() {
    let room = playing_room(5.0, 0.0);
    let mut c = ready_client(room.clone());
    c.set_buffering(true);
    assert_eq!(c.phase, Phase::Buffering);
    assert_eq!(c.room.as_ref().unwrap(), &room);
    assert!(c.emit(Role::Controller, Cmd::Pause).is_some());
}

#[test]
fn watcher_cannot_move_room() {
    let room = playing_room(5.0, 0.0);
    assert_eq!(room.handle(Role::Watcher, &Cmd::Pause, 1.0), Err(Denied));
    assert_eq!(room.handle(Role::Watcher, &Cmd::Seek { target: 0.0 }, 1.0), Err(Denied));
    let c = ready_client(room);
    assert!(c.emit(Role::Watcher, Cmd::Pause).is_none());
}

#[test]
fn wait_mode_pauses_and_resumes() {
    let room = playing_room(5.0, 0.0);
    let mut g = WaitGate::new(true, 5.0, 30.0);
    assert_eq!(g.tick(&room, &[2.0], 10.0), None);                       // below threshold
    assert_eq!(g.tick(&room, &[6.0], 11.0), Some(Cmd::PauseWaiting));
    let paused = room.apply(&Cmd::PauseWaiting, 11.0);
    assert!(paused.waiting && !paused.playing);
    assert_eq!(g.tick(&paused, &[4.0], 20.0), None);                     // still waiting
    assert_eq!(g.tick(&paused, &[], 21.0), Some(Cmd::Play));             // everybody ready
    // timeout path
    let mut g2 = WaitGate::new(true, 5.0, 30.0);
    g2.tick(&room, &[9.0], 0.0);
    assert_eq!(g2.tick(&paused, &[40.0], 31.0), Some(Cmd::Play));
    assert!(!WaitGate::new(false, 5.0, 30.0).tick(&room, &[99.0], 1.0).is_some());
}

#[test]
fn local_mode_emits_no_sync_events() {
    let mut c = Client::new(Mode::Local);
    c.on_connected();
    for cmd in [Cmd::Play, Cmd::Pause, Cmd::Seek { target: 3.0 }, Cmd::SetRate(1.5)] {
        assert!(c.emit(Role::Controller, cmd).is_none());
    }
}

#[test]
fn seek_debouncer_collapses_bursts() {
    let mut d = SeekDebouncer::new(0.4);
    d.push(10.0, 0.0); d.push(20.0, 0.1); d.push(30.0, 0.2);
    assert_eq!(d.poll(0.3), None);
    assert_eq!(d.poll(0.7), Some(30.0));
    assert_eq!(d.poll(1.0), None);
}

#[test]
fn clock_sim_p95_error_under_150ms() {
    // deterministic LCG; 200 clients with random clock offsets and asymmetric jitter.
    let mut x: u64 = 0x2545F4914F6CDD1D;
    let mut rnd = move || { x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((x >> 33) as f64) / ((1u64 << 31) as f64) };
    let mut errs = vec![];
    for _ in 0..200 {
        let true_off = (rnd() - 0.5) * 1000.0; // server = client + true_off
        let mut c = ClockSync::default();
        for i in 0..8 {
            let t0 = i as f64 * 2.0;
            let up = 0.02 + rnd() * 0.10;   // 20–120 ms
            let down = 0.02 + rnd() * 0.10;
            let ts = t0 + up + true_off;
            let t1 = t0 + up + down;
            c.push(t0, ts, t1);
        }
        errs.push((c.best().unwrap().offset - true_off).abs());
    }
    errs.sort_by(|a, b| a.total_cmp(b));
    let p95 = errs[(errs.len() as f64 * 0.95) as usize];
    assert!(p95 < 0.15, "p95 = {p95}");
}
