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
- Both recipes' `.SRCINFO` files

After that PR is merged, the release workflow creates the `vX.Y.Z` tag, builds
release artifacts, and publishes a GitHub release.

### AUR publishing

The release workflow prepares `instantwm` (source) and `instantwm-bin`
(prebuilt x86_64), then installs and smoke-tests the binary package on Arch.
The `aur-recipes` workflow artifact contains both checksummed recipes and their
`.SRCINFO` files. GitHub release reruns preserve the binary archive checksum.

For the first `instantwm-bin` submission, push `PKGBUILD` and `.SRCINFO`
from `packaging/arch-bin` for its pinned release, or from the workflow artifact,
to the AUR package's `master` branch.
Give the AUR account behind the CI SSH key push access to both packages, then
set `AUR_SSH_PRIVATE_KEY` as a repository Actions secret. Eligible releases
from `main` or protected tags publish to the AUR once the secret is set.

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
