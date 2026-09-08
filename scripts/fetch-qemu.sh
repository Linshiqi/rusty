#!/usr/bin/env bash
# Fetch rusty's own QEMU build — the one with the GPIO model — into
# crates/rusty-app/bundled/, where `cargo tauri build` packages it as a
# resource. The release workflow runs this before every build, so a fresh
# install simulates without a download and without Espressif's stock build
# ever answering for a pin. Run it once locally to make `cargo tauri dev` use
# ours too; without it the app falls back to the data directory and then to
# the download ladder, exactly as before.
#
# A no-op, said aloud, on a platform the qemu-v3 release has no build for
# (Intel macOS today): the app then fetches Espressif's on demand.
set -euo pipefail

tag="${RUSTY_QEMU_TAG:-qemu-v3}"
case "$(uname -s)-$(uname -m)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) platform=x86_64-w64-mingw32 ;;
  Darwin-arm64)                  platform=aarch64-apple-darwin ;;
  Linux-x86_64)                  platform=x86_64-linux-gnu ;;
  *)
    echo "fetch-qemu: no rusty QEMU build for $(uname -s)-$(uname -m); the app will fall back to the download" >&2
    exit 0
    ;;
esac

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/crates/rusty-app/bundled"
asset="qemu-rusty-${tag#qemu-}-$platform.tar.xz"
url="https://github.com/Linshiqi/rusty/releases/download/$tag/$asset"

mkdir -p "$dest"
echo "fetch-qemu: $url"
curl -fsSL --retry 3 -o "$dest/$asset" "$url"
rm -rf "$dest/qemu"
tar -xf "$dest/$asset" -C "$dest"
rm -f "$dest/$asset"

# Only the ESP ROMs travel. QEMU's share directory carries firmware for
# every machine it can model — openbios-sparc32, PowerPC and s390 images,
# some of them ELF files for other architectures — and the AppImage bundler
# walks the app's resources deploying the dependencies of every ELF it
# finds, so a SPARC executable in there ended the Linux build with
# "failed to run linuxdeploy". Three megabytes of ROMs stand in for forty of
# firmware nothing here can boot.
find "$dest/qemu/share/qemu" -mindepth 1 -maxdepth 1 ! -name 'esp32*' -exec rm -rf {} +

# The platform the binaries are for, read at runtime: a universal macOS
# bundle carries one architecture's QEMU, and the other must not try it.
printf '%s\n' "$platform" > "$dest/qemu/PLATFORM"

if grep -q '\[rusty:gpio@' "$dest"/qemu/bin/qemu-system-riscv32*; then
  echo "fetch-qemu: bundled rusty's QEMU ($tag, $platform) into $dest/qemu"
else
  echo "fetch-qemu: the archive is not rusty's build — no GPIO model marker" >&2
  exit 1
fi
