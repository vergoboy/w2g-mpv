#!/usr/bin/env bash
# Builds the w2g sync plugin (build/w2g/w2g.so) against the mpv headers in this tree.
set -euo pipefail
cd "$(dirname "$0")/.."
( cd w2g/sync && cargo build --release )
mkdir -p build/w2g
cc -O2 -Wall -Wextra -shared -fPIC -Iinclude -Iw2g/sync/include \
   w2g/plugin/w2g_plugin.c w2g/sync/target/release/libw2g_sync.a \
   -o build/w2g/w2g.so -lpthread -ldl -lm
echo "built build/w2g/w2g.so"
