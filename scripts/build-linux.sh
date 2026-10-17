#!/usr/bin/env bash
# Reproducible out-of-tree build of the w2g mpv fork (Linux).
#   scripts/build-linux.sh ci      headless: no GL/Vulkan/windowing (compile + libmpv check)
#   scripts/build-linux.sh full    desktop: GL, EGL, X11, Wayland, Vulkan, ALSA/Pulse/PipeWire
# Needs: meson>=1.3, ninja, ffmpeg dev libs, libass, and libplacebo >= 7.360.1
# (set PKG_CONFIG_PATH if you built libplacebo into a private prefix).
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${1:-ci}"
B="build/$PROFILE"
COMMON=(-Dgpl=true -Dlibmpv=true -Dcplayer=true -Dbuild-date=false -Dtests=false)
case "$PROFILE" in
  ci)   OPTS=(-Dauto_features=disabled -Dgl=disabled) ;;
  full) OPTS=(-Dauto_features=auto) ;;
  *) echo "profile must be ci|full" >&2; exit 2 ;;
esac
rm -rf "$B"
meson setup "$B" "${COMMON[@]}" "${OPTS[@]}"
meson compile -C "$B"
"$B/mpv" --version | head -2
ls -la "$B"/libmpv.so* | awk '{print $5, $9}'
