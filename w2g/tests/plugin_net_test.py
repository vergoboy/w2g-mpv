# End-to-end: w2g-server + two real mpv instances (controller / watcher) synchronised through it.
# Uses real signed room tokens (w2g-token). Needs /tmp/t.wav and `cargo build` in w2g/server. Run: W2G_MPV=build/ci/mpv python3 w2g/tests/plugin_net_test.py
import json, os, socket, subprocess, time
MPV = os.environ.get('W2G_MPV', 'build/ci/mpv')
PORT = '18765'
LIB = os.environ.get('W2G_LIBDIR', os.environ.get('LD_LIBRARY_PATH', ''))
SECRET = 'test-secret-test-secret-test-secret-0123'
SRV = 'w2g/server/target/debug/'
def token(sub, role):
    return subprocess.check_output([SRV + 'w2g-token', sub, 'r1', role, '600'], env=dict(os.environ, W2G_SECRET=SECRET), text=True).strip()
srv = subprocess.Popen([SRV + 'w2g-server'], env=dict(os.environ, W2G_BIND='127.0.0.1:' + PORT, W2G_SECRET=SECRET),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.5)

class Player:
    def __init__(self, name, token, role):
        self.sock = f'/tmp/w2g-net-{name}.sock'
        if os.path.exists(self.sock): os.remove(self.sock)
        env = dict(os.environ, LD_LIBRARY_PATH=LIB, W2G_URL=f'ws://127.0.0.1:{PORT}', W2G_TOKEN=token,
                   W2G_ROOM='r1', W2G_ROLE=role)
        self.p = subprocess.Popen([MPV, '--no-config', '--vo=null', '--ao=null', '--idle=yes', '--pause',
            f'--input-ipc-server={self.sock}', '--script=build/w2g/w2g.so', '/tmp/t.wav'],
            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(50):
            if os.path.exists(self.sock): break
            time.sleep(0.1)
        s = socket.socket(socket.AF_UNIX); s.connect(self.sock); self.f = s.makefile('rw')
    def cmd(self, *a):
        self.f.write(json.dumps({'command': list(a)}) + '\n'); self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if 'error' in r: return r.get('data')
    def pos(self): return self.cmd('get_property', 'time-pos')
    def paused(self): return self.cmd('get_property', 'pause')
    def stop(self):
        try: self.cmd('quit')
        except Exception: pass
        self.p.wait(timeout=10)

ctl = Player('ctl', token('alice', 'controller'), 'controller'); wat = Player('wat', token('bob', 'watcher'), 'watcher')
res = {}
try:
    time.sleep(2.0)                                   # connect + clock samples + first state
    ctl.cmd('set_property', 'pause', False); time.sleep(2.0)       # controller presses play
    res['play'] = (ctl.paused(), wat.paused(), round(ctl.pos() - wat.pos(), 2))
    ctl.cmd('seek', '60', 'absolute+exact'); time.sleep(1.5)       # controller seeks
    res['seek'] = (round(ctl.pos(), 1), round(wat.pos(), 1))
    wat.cmd('set_property', 'pause', True); time.sleep(1.0)        # watcher tries to pause locally
    res['watcher_pause_reverted'] = wat.paused() is False
    ctl.cmd('set_property', 'pause', True); time.sleep(1.5)        # controller pauses for everyone
    res['pause_all'] = (ctl.paused(), wat.paused(), round(abs(ctl.pos() - wat.pos()), 2))
    wat.cmd('seek', '10', 'absolute+exact'); time.sleep(2.0)       # watcher seeks locally -> pulled back
    res['watcher_seek_pulled_back'] = round(wat.pos(), 1)
finally:
    ctl.stop(); wat.stop(); srv.terminate()
print(json.dumps(res))
assert res['play'][0] is False and res['play'][1] is False and abs(res['play'][2]) < 0.5, res['play']
assert 60 <= res['seek'][0] <= 62.5 and abs(res['seek'][0] - res['seek'][1]) < 0.6, res['seek']
assert res['watcher_pause_reverted'], res
assert res['pause_all'][0] is True and res['pause_all'][1] is True and res['pause_all'][2] < 0.6, res['pause_all']
assert abs(res['watcher_seek_pulled_back'] - res['seek'][0]) < 4, res['watcher_seek_pulled_back']
print('OK')
