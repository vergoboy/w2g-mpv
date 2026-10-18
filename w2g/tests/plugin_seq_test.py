# Regression: duplicate seq must be ignored; a new seq must apply at once (no cooldown wait).
# Needs /tmp/t.wav. Run: W2G_MPV=build/ci/mpv python3 w2g/tests/plugin_seq_test.py
import json, os, socket, subprocess, time
SOCK = '/tmp/w2g-seq.sock'
if os.path.exists(SOCK): os.remove(SOCK)
env = dict(os.environ, LD_LIBRARY_PATH=os.environ.get('W2G_LIBDIR', os.environ.get('LD_LIBRARY_PATH', '')))
p = subprocess.Popen([os.environ.get('W2G_MPV', 'build/ci/mpv'), '--no-config', '--vo=null', '--ao=null', '--idle=yes',
    '--pause', f'--input-ipc-server={SOCK}', '--script=build/w2g/w2g.so', '/tmp/t.wav'],
    env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(50):
    if os.path.exists(SOCK): break
    time.sleep(0.1)
s = socket.socket(socket.AF_UNIX); s.connect(SOCK); f = s.makefile('rw')
def cmd(*a):
    f.write(json.dumps({'command': list(a)}) + '\n'); f.flush()
    while True:
        r = json.loads(f.readline())
        if 'error' in r: return r.get('data')
def probe(seq, playing, pos, wait=1.0):
    cmd('script-message-to', 'w2g', 'w2g-state', str(seq), str(playing), str(pos), str(time.monotonic()), '1')
    time.sleep(wait)
    return round(cmd('get_property', 'time-pos'), 1), cmd('get_property', 'pause')
time.sleep(1.0)
r1 = probe(1, 1, 10);  r2 = probe(2, 0, 30);  r3 = probe(2, 1, 5);  r4 = probe(3, 1, 5, wait=1.0)
cmd('quit'); p.wait(timeout=10)
print(r1, r2, r3, r4)
assert 10 <= r1[0] <= 12 and r1[1] is False, r1
assert abs(r2[0] - 30) < 0.5 and r2[1] is True, r2
assert abs(r3[0] - 30) < 0.5 and r3[1] is True, f'duplicate seq must be ignored: {r3}'
assert 5 <= r4[0] <= 7.5 and r4[1] is False, f'new seq must apply without cooldown: {r4}'
print('OK')
