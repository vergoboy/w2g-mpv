# w2g/ — fork-specific code (everything that is not upstream mpv lives here)

- `sync/`  Rust crate `w2g-sync`: room state, clock, drift, echo guard (pure, tested).
           Next: C ABI + hook into the player core (see docs/adr/0001).
- `plugin/` C plugin loaded with `--script=build/w2g/w2g.so`: applies RoomState inside mpv (pause, drift, seek, echo guard, watcher revert).
- `server/` room sync server (WebSocket, std threads): rooms, roles, clock pong, periodic idempotent state. AUTH IS A PLACEHOLDER.
- `tests/`  `plugin_ipc_test.py`: real mpv + plugin over IPC (controller and watcher).
