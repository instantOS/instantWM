# Contributing

## Version Bumps and Releases

instantWM is a binary crate and is not published to crates.io. Releases are
prepared by GitHub Actions instead of release-plz.

The version bump workflow reads commit messages since the latest `v*` tag and
opens a release PR that updates:

- `Cargo.toml`
- `Cargo.lock`
- `CHANGELOG.md`
- `packaging/arch/PKGBUILD`
- `packaging/arch-bin/PKGBUILD`

After that PR is merged, the release workflow creates the `vX.Y.Z` tag, builds
release artifacts, and publishes a GitHub release.

### Commit Message Format

Use Conventional Commit-style subjects when a change should influence the next
version:

```text
fix: correct focus ordering
feat: add a new instantwmctl command
feat!: change the IPC response format
```

Automatic bumping uses these rules:

- `fix:` and most other code changes create a patch bump.
- `feat:` creates a minor bump.
- `type!:` creates a major bump.
- A commit body containing `BREAKING CHANGE` creates a major bump.
- `chore:`, `ci:`, `docs:`, `style:`, and `test:` do not trigger a release in
  automatic mode.

Because instantWM is a binary, breaking changes cannot be reliably detected from
Rust public API changes. Mark user-facing compatibility breaks explicitly with
`!` or `BREAKING CHANGE`.

Examples of user-facing compatibility breaks include:

- changing the config format
- removing or renaming CLI flags or `instantwmctl` commands
- changing IPC request or response shapes
- changing session or packaging behavior that users or downstream packages rely on

Manual release PRs can also be started from the `Version bump PR` workflow with
an explicit `patch`, `minor`, or `major` bump.
