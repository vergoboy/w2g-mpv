# w2g/ — fork-specific code (everything that is not upstream mpv lives here)

- `sync/`  Rust crate `w2g-sync`: room state, clock, drift, echo guard (pure, tested).
           Next: C ABI + hook into the player core (see docs/adr/0001).
