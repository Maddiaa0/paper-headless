# Releasing paper-headless

Releases are built by `dist` from version tags. The workflow builds Linux
binaries for x86_64 and arm64 (glibc and musl). It creates the shell, npm,
and Homebrew installers and publishes checksums and GitHub attestations. It
then publishes to npm, Homebrew, and crates.io. `install.sh` downloads the
shell installer from the latest release.

## One-time setup

Add these Actions secrets to this repository:

1. `CARGO_REGISTRY_TOKEN`: a crates.io token that can publish `paper-headless`.
2. `HOMEBREW_TAP_TOKEN`: a token that can write `Maddiaa0/homebrew-tap`. The
   tap must be public so `brew install` works.
3. `NPM_TOKEN`: an npm token that can publish `@maddiaa0/paper-headless`.

## Cut a release

1. Update the version in `Cargo.toml` and run the checks:

   ```console
   $ cargo fmt --check
   $ cargo clippy --locked --all-targets -- -D warnings
   $ btt check
   $ cargo test --locked
   $ cargo publish --locked --dry-run
   $ dist plan
   ```

2. Merge the version change to `main`, then tag that commit with the same version:

   ```console
   $ git tag v0.1.0
   $ git push origin v0.1.0
   ```

If only a publish job fails, re-run that job from the Actions page. Do not
re-tag.

`dist` owns `.github/workflows/release.yml`. Change release settings in
`dist-workspace.toml`, then run `dist generate`.
