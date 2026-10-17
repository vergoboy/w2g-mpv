# ADR 0001 — Fork mpv; sync lives inside the player, rich UI lives beside it

Status: accepted (owner decision, 2026-10-18). Supersedes the old "mpv only as a separate
process / read-only source / LGPL-only" rule (ADR 0002 of vergoboy-stream).

## Context
The product is a synchronized watch-party player. The owner will release the project as
open source, so GPL is acceptable. mpv already provides playback, tracks, subtitles,
HDR/10-bit, scripting (Lua/JS), audio filters and a client API.

## Decision
1. This repository is a fork of mpv-player/mpv (upstream base `01e2533`, remote `upstream`).
   Fork code lives in `w2g/`; changes to upstream files are kept minimal, each one a
   separate commit prefixed `w2g:` so rebases onto upstream stay cheap.
2. Build is the full GPL build (`-Dgpl=true`). The project licence becomes GPLv2+ (as mpv).
3. Sync (room state, clock, drift, echo guard) is the Rust crate `w2g-sync`, linked into
   the fork through a C ABI. The player stays the authority on playback; the server
   stays the authority on room intent.
4. Player UI (controls, track/subtitle menus, OSD) is Lua + ASS following the Figma
   "VLC Redesign" frames (pending exports). Pages that mpv's OSD cannot do well
   (auth, archive, full chat, owner panel, RTL forms) live in a separate light UI
   process that talks to the player over the client API. No second video surface.
5. Noise suppression is an mpv audio filter (`af`) running a neural model with a
   per-frame time budget. Client-side bypass of VPN/tun is the Rust networking layer.

## Rules
- Keep upstream mergeable: rebase, do not squash upstream history.
- One logical change per commit; every non-trivial change has a test or a stated reason.
- Anything that cannot be verified in CI is listed in docs/QA_MANUAL.md.

## Consequences
+ one binary and one process for playback + sync; no IPC hop on the hot path.
- we own upstream rebases; mitigated by keeping the diff small and isolated in `w2g/`.
