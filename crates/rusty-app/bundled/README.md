# Bundled tools

What the installer ships beside the app, found by `rusty_embed::tools` after
the data directory's own `tools/` and before PATH.

`qemu/` is rusty's build of Espressif's QEMU with the GPIO device model —
`scripts/fetch-qemu.sh` puts it here from the `qemu-v*` release, and the
release workflow runs that script before every build. It is not committed:
sixty megabytes of binaries belong in a release asset, not in git. Without
it (a checkout that never ran the script) the app falls back to the data
directory and then to the download ladder, as it always did.

`qemu/PLATFORM` names the target the binaries were built for; a bundle for
another architecture is ignored rather than tried.
