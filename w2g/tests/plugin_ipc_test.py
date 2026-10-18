# Integration test: real mpv + w2g.so driven over JSON IPC. Needs /tmp/t.wav (120 s of silence).
# Run: scripts/build-linux.sh ci && scripts/build-w2g.sh && ROLE=watcher python3 w2g/tests/plugin_ipc_test.py
import json, socket, subprocess, time, os, sys
SOCK='/tmp/w2g-mpv.sock'
if os.path.exists(SOCK): os.remove(SOCK)
env=dict(os.environ, LD_LIBRARY_PATH=os.environ.get('W2G_LIBDIR', os.environ.get('LD_LIBRARY_PATH','')), W2G_ROLE=os.environ.get('ROLE','controller'))
p=subprocess.Popen([os.environ.get('W2G_MPV','build/ci/mpv'),'--no-config','--vo=null','--ao=null','--idle=yes','--pause',
   f'--input-ipc-server={SOCK}','--script=build/w2g/w2g.so','/tmp/t.wav'],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
for _ in range(50):
    if os.path.exists(SOCK): break
    time.sleep(0.1)
s=socket.socket(socket.AF_UNIX); s.connect(SOCK); f=s.makefile('rw')
def cmd(*a):
    f.write(json.dumps({'command':list(a)})+'\n'); f.flush()
    while True:
        r=json.loads(f.readline())
        if 'error' in r: return r.get('data')
def state(seq,playing,pos,rate=1.0): cmd('script-message-to','w2g','w2g-state',str(seq),str(playing),str(pos),str(time.monotonic()),str(rate))
time.sleep(1.0)
res={}
state(1,1,10.0); time.sleep(3.0)
res['play_pos_after_3s']=cmd('get_property','time-pos'); res['paused_1']=cmd('get_property','pause')
state(0,0,50.0); time.sleep(0.6)       # stale seq: must be ignored
res['stale_ignored_pos']=cmd('get_property','time-pos'); res['paused_stale']=cmd('get_property','pause')
state(2,0,30.0); time.sleep(1.2)       # pause at 30
res['paused_state_pause']=cmd('get_property','pause'); res['paused_state_pos']=cmd('get_property','time-pos')
cmd('set_property','pause',False); time.sleep(0.8); res['pause_after_user_play']=cmd('get_property','pause')   # user presses play -> would emit, then room pulls back
res['speed']=cmd('get_property','speed')
cmd('quit'); out=p.communicate(timeout=10)[0]
res['emits']=[l for l in out.splitlines() if 'w2g-emit' in l]
print(json.dumps(res,indent=1)); print('--- log tail'); print('\n'.join(out.splitlines()[-4:]))
