#!/usr/bin/env bash
# Fetch the tools the installer ships beside the app into
# crates/rusty-app/bundled/, where `cargo tauri build` packages them as
# resources. The release workflow runs this before every build; run it once
# locally and `cargo tauri dev` uses them too.
#
# **What this is for.** A fresh install used to be a workbench that could not
# simulate, debug or flash until the user had found and fetched four things.
# Everything here is a binary rusty drives, so shipping them is the difference
# between "installed" and "usable": with these in the bundle, a RISC-V Rust
# project builds, boots in the simulator, stops on a breakpoint and flashes,
# with no download at all.
#
# **What is deliberately not here.** Rust itself — rustup, cargo, the standard
# library, and espup's Xtensa fork. Those belong in the user's own `~/.cargo`
# and rustup home, not in rusty's resources: which toolchain a project needs is
# decided by its `rust-toolchain.toml`, rustup is the only thing that installs
# them correctly, and a copy frozen into an installer goes stale in six weeks.
# rusty's first-run setup screen asks for those, which is the honest shape for
# a dependency the user has to own. The C cross compilers are out too, at four
# hundred megabytes each for a case — C interop, and Xtensa linking, which
# needs espup anyway — that the bundle could not complete on its own.
#
# The bundle is a *fallback*: `rusty_embed::tools::find` reaches it only after
# the data directory, cargo's bin and PATH, so a copy the user installed on
# purpose still wins. The one exception is rusty's own QEMU, which is not
# interchangeable with a stock build wearing the same name.
#
# A tool with no build for this platform is said aloud and skipped, not
# failed: Espressif publishes no macOS esp-gdb, and on that platform the
# debugger is CodeLLDB, which is here.
set -euo pipefail

case "$(uname -s)-$(uname -m)" in
  MINGW*|MSYS*|CYGWIN*|Windows*) platform=x86_64-w64-mingw32 ;;
  Darwin-arm64)                  platform=aarch64-apple-darwin ;;
  Linux-x86_64)                  platform=x86_64-linux-gnu ;;
  *)
    echo "bundle-tools: nothing is published for $(uname -s)-$(uname -m); the app will fetch on demand" >&2
    exit 0
    ;;
esac

# Pinned, every one of them. A tool version that comes from `latest` is not a
# version: the build that ships is then whatever the release page held that
# morning, and a bug report names an installer rather than a binary.
qemu_tag="${RUSTY_QEMU_TAG:-qemu-v3}"
gdb_release=esp-gdb-v14.2_20240403
gdb_version=14.2_20240403
espflash_version=v4.0.1
codelldb_version=v1.12.3

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/crates/rusty-app/bundled"
mkdir -p "$dest"

# Windows' own tar, by absolute path: a bare `tar` there is usually Git's GNU
# tar, which reads `E:/…` as a remote host and dies with "Cannot connect to
# E:". System32's is bsdtar, which reads .zip, .tar.gz and .tar.xz alike.
if [ "$platform" = x86_64-w64-mingw32 ]; then
  TAR="/c/Windows/System32/tar.exe"
else
  TAR=tar
fi

# Fetch a URL, telling "this platform has no such build" from "the download
# failed". A 404 leaves an empty file and succeeds; anything else that does
# not arrive intact fails, after retrying — including transfer errors, which
# `--retry` alone does not cover.
download() {
  local url="$1" into="$2" code
  : > "$into"
  code="$(curl -sIL -o /dev/null -w '%{http_code}' --retry 3 --retry-delay 2 "$url" || echo 000)"
  if [ "$code" = 404 ]; then
    return 0
  fi
  curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors -o "$into" "$url"
}

# family, url, and a path under the family that must exist afterwards.
fetch() {
  local family="$1" url="$2" proof="$3"
  local archive="$dest/.$family.download"

  echo "bundle-tools: $family <- $url"
  # "Not published" and "the download failed" are different answers and only
  # one of them may be shrugged off. A transfer error read as the first is an
  # installer quietly missing a tool, blamed on a platform that publishes it
  # perfectly well — which is what this did on the first run, twice, on a
  # network that drops TLS connections. So the status code decides, and
  # anything that is not a clean 404 or a clean download stops the build.
  if ! download "$url" "$archive"; then
    echo "::error::bundle-tools: $family could not be downloaded for $platform"
    exit 1
  fi
  if [ ! -s "$archive" ]; then
    echo "bundle-tools: $family is not published for $platform; the app will fetch or refuse at runtime" >&2
    rm -f "$archive"
    return 0
  fi
  rm -rf "${dest:?}/$family"
  mkdir -p "$dest/$family"
  # Every archive unpacks *into* its family directory, which is the shape
  # `tools::find` walks: `<family>/bin/<exe>` and `<family>/<exe>`.
  "$TAR" -xf "$archive" -C "$dest/$family"
  rm -f "$archive"
  if [ ! -e "$dest/$family/$proof" ]; then
    # A directory-level archive unpacks one level deep; flatten it so the
    # ladder finds the binary where it looks.
    local inner
    inner="$(find "$dest/$family" -mindepth 1 -maxdepth 1 -type d | head -1)"
    if [ -n "$inner" ] && [ -e "$inner/$proof" ]; then
      mv "$inner"/* "$dest/$family"/ 2>/dev/null || true
      rmdir "$inner" 2>/dev/null || true
    fi
  fi
  if [ ! -e "$dest/$family/$proof" ]; then
    echo "bundle-tools: $family unpacked without $proof — the archive is not the shape this expects" >&2
    exit 1
  fi
  echo "bundle-tools: $family ready"
}

# ── rusty's QEMU ────────────────────────────────────────────────────────────
# Its own step: the archive already carries a `qemu/` directory, and the
# marker check below is what tells our build from Espressif's.
qemu_asset="qemu-rusty-${qemu_tag#qemu-}-$platform.tar.xz"
qemu_url="https://github.com/Linshiqi/rusty/releases/download/$qemu_tag/$qemu_asset"
echo "bundle-tools: qemu <- $qemu_url"
curl -fsSL --retry 3 -o "$dest/$qemu_asset" "$qemu_url"
rm -rf "$dest/qemu"
"$TAR" -xf "$dest/$qemu_asset" -C "$dest"
rm -f "$dest/$qemu_asset"

# Only the ESP ROMs travel. QEMU's share directory carries firmware for
# every machine it can model — openbios-sparc32, PowerPC and s390 images,
# some of them ELF files for other architectures — and the AppImage bundler
# walks the app's resources deploying the dependencies of every ELF it
# finds, so a SPARC executable in there ended the Linux build with
# "failed to run linuxdeploy". Three megabytes of ROMs stand in for forty of
# firmware nothing here can boot.
find "$dest/qemu/share/qemu" -mindepth 1 -maxdepth 1 ! -name 'esp32*' -exec rm -rf {} +

if grep -q '\[rusty:gpio@' "$dest"/qemu/bin/qemu-system-riscv32*; then
  echo "bundle-tools: qemu ready ($qemu_tag) — this is rusty's build"
else
  echo "bundle-tools: the archive is not rusty's build — no GPIO model marker" >&2
  exit 1
fi

# ── the debuggers ───────────────────────────────────────────────────────────
# Espressif publishes esp-gdb for Windows and Linux and not for macOS, where
# `fetch` says so and moves on: CodeLLDB below is that platform's debugger,
# and it is the one an `-msvc` host needs anyway, since gdb cannot read a PDB.
gdb_base="https://github.com/espressif/binutils-gdb/releases/download/$gdb_release"
#
# Each proof names the binary *rusty asks for*, which is not the family's own
# name for the Xtensa one: `simulate::find_gdb` looks for
# `xtensa-esp32-elf-gdb`, because Espressif builds that family per chip and
# ships no plain `xtensa-esp-elf-gdb`. Proving the archive by its directory
# name instead would pass on an archive rusty cannot use.
case "$platform" in
  x86_64-w64-mingw32) gdb_suffix="x86_64-w64-mingw32.zip";  gdb_ext=.exe ;;
  x86_64-linux-gnu)   gdb_suffix="x86_64-linux-gnu.tar.gz"; gdb_ext= ;;
  *)                  gdb_suffix="" ;;
esac
if [ -n "$gdb_suffix" ]; then
  fetch riscv32-esp-elf-gdb "$gdb_base/riscv32-esp-elf-gdb-$gdb_version-$gdb_suffix" \
        "bin/riscv32-esp-elf-gdb$gdb_ext"
  fetch xtensa-esp-elf-gdb "$gdb_base/xtensa-esp-elf-gdb-$gdb_version-$gdb_suffix" \
        "bin/xtensa-esp32-elf-gdb$gdb_ext"
else
  echo "bundle-tools: Espressif publishes no esp-gdb for $platform; CodeLLDB is the debugger there"
fi

# ── the flasher ─────────────────────────────────────────────────────────────
# `cargo install espflash` builds it in a couple of minutes on a machine that
# has Rust; the prebuilt is here so a machine that has just been installed on
# does not have to.
case "$platform" in
  x86_64-w64-mingw32) espflash_target=x86_64-pc-windows-msvc;    espflash_exe=espflash.exe ;;
  x86_64-linux-gnu)   espflash_target=x86_64-unknown-linux-gnu;  espflash_exe=espflash ;;
  aarch64-apple-darwin) espflash_target=aarch64-apple-darwin;    espflash_exe=espflash ;;
esac
fetch espflash \
  "https://github.com/esp-rs/espflash/releases/download/$espflash_version/espflash-$espflash_target.zip" \
  "$espflash_exe"

# ── the LLDB adapter ────────────────────────────────────────────────────────
# A `.vsix` is a zip whose payload sits under `extension/`, so it keeps a
# directory of its own — the adapter loads its LLDB from the `lldb/` beside
# it, and flattening that would break it. `host_adapters` knows this shape.
#
# **Not on Linux**, and the reason is the one that cost a release already:
# the AppImage bundler walks every ELF among the app's resources and deploys
# its dependencies, which is how QEMU's firmware directory ended the Linux
# build with "failed to run linuxdeploy". CodeLLDB carries a hundred and
# thirty megabytes of host LLDB, and Linux is the one platform where
# Espressif publishes esp-gdb — so that desktop already has a debugger in the
# bundle and this stays a download there. Windows and macOS need it: macOS
# has no esp-gdb published at all, and gdb cannot read the PDB an `-msvc`
# host puts its debug information in.
case "$platform" in
  x86_64-w64-mingw32)   codelldb_asset=codelldb-win32-x64.vsix ;;
  aarch64-apple-darwin) codelldb_asset=codelldb-darwin-arm64.vsix ;;
  *)                    codelldb_asset="" ;;
esac
if [ -z "$codelldb_asset" ]; then
  echo "bundle-tools: CodeLLDB stays a download on $platform — esp-gdb is bundled there"
  printf '%s\n' "$platform" > "$dest/PLATFORM"
  echo "bundle-tools: $platform bundle is"
  du -sh "$dest"/* 2>/dev/null || true
  exit 0
fi
echo "bundle-tools: codelldb <- $codelldb_asset"
if curl -fsSL --retry 3 -o "$dest/.codelldb.vsix" \
     "https://github.com/vadimcn/codelldb/releases/download/$codelldb_version/$codelldb_asset"; then
  rm -rf "$dest/codelldb"
  mkdir -p "$dest/codelldb"
  "$TAR" -xf "$dest/.codelldb.vsix" -C "$dest/codelldb"
  rm -f "$dest/.codelldb.vsix"
  echo "bundle-tools: codelldb ready"
else
  echo "bundle-tools: no CodeLLDB build for $platform" >&2
fi

# The platform the binaries are for, read at runtime: a universal macOS
# bundle carries one architecture's tools, and the other must not try them.
# At the bundle's root rather than under `qemu/`, because the bundle is no
# longer only QEMU.
printf '%s\n' "$platform" > "$dest/PLATFORM"

echo "bundle-tools: $platform bundle is"
du -sh "$dest"/* 2>/dev/null || true
