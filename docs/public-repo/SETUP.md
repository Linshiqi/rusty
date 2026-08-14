# Setting up the public releases repository

Once. This directory is the content of `Linshiqi/rusty-releases`; the private
repository builds into it and never pushes source there.

## 1. Create it

A new **public** repository named `rusty-releases`, with Issues enabled and
nothing else — no README from the wizard, the one here replaces it.

## 2. Seed it

The release action attaches a release to a tag, and a tag needs a commit to
point at. An empty repository has none, so publishing into it fails with
something unhelpful about a missing reference.

```bash
git clone https://github.com/Linshiqi/rusty-releases && cd rusty-releases
# copy README.md and .github/ from docs/public-repo/ in the private repo
git add -A && git commit -m "Downloads and issues" && git push
```

## 3. The token that lets the private repo publish here

`GITHUB_TOKEN` is scoped to the repository the workflow runs in, so it cannot
write here. Make a **fine-grained** personal access token:

- Settings → Developer settings → Personal access tokens → Fine-grained
- Repository access: **only** `Linshiqi/rusty-releases`
- Permissions: Repository permissions → **Contents: Read and write**
- Nothing else. This token can publish releases in one public repository and
  do nothing anywhere else, which is the whole point of scoping it.

Then store it in the **private** repository (the one that builds):

```bash
gh secret set RELEASES_TOKEN --repo Linshiqi/rusty
```

or Settings → Secrets and variables → Actions → New repository secret, named
`RELEASES_TOKEN`.

## 4. Check what actually crossed over

After the next tag, the public release should carry installers and the
`CHANGELOG.md` section — and nothing else. Specifically **not** generated
release notes: those are built from the private repository's commit messages,
which describe internals in detail. The workflow sets
`generate_release_notes: false` for exactly this reason; if a future edit
turns it back on, the next release publishes the private history.

## Signing (optional, separate)

With `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` also
in the private repository's secrets, each build additionally emits signed
updater artifacts and a `latest.json` feed. Nothing in the app consumes that
feed yet — `update::check` compares versions and opens the release page — so
this is preparation, not a working in-app updater.
