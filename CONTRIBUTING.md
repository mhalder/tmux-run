# Contributing

## Development

Build and test with:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

The minimum supported Rust version is 1.88, and the crate uses edition 2024.

## Policy

tmux-run is dependency-free: standard library only. A new dependency needs a strong justification before it is added.

Commit messages follow the [Conventional Commits](https://www.conventionalcommits.org/) format.

## Versioning and releases

Releases follow [Semantic Versioning](https://semver.org/), driven by
conventional commits and [release-plz](https://release-plz.dev/). The version
lives in `Cargo.toml` and nowhere else.

The public API of a command-line tool is its command line, its exit codes, and
the files it installs:

- **Major** — a script that worked stops working: a subcommand, flag, or exit
  code meaning is removed or changed, the `__DONE__:<status>` marker changes, a
  shell is dropped from `completion`, or the `skill` install path moves.
- **Minor** — a subcommand, flag, shell, or the embedded skill is added, and
  existing invocations keep working.
- **Patch** — a fix that leaves the command line, the exit codes, and the log
  format intact.

Before 1.0 a breaking change bumps the minor version, as SemVer allows.

The commit type decides the bump, and whether a release happens at all:

| Commit | Release |
| --- | --- |
| `feat:` | minor |
| `fix:` | patch |
| `perf:`, `refactor:` | patch, in "Changed" |
| `revert:` | patch, in "Removed" |
| any type with `!`, or a `BREAKING CHANGE:` footer | the breaking change, whatever the type |
| `build:`, `chore:`, `ci:`, `docs:`, `style:`, `test:` | no release, no changelog entry |

Nobody bumps the version by hand. A merged commit of a releasing type makes
release-plz open or update a release PR that bumps `Cargo.toml` and
`Cargo.lock` and writes the `CHANGELOG.md` section; merging that PR tags
`vX.Y.Z`, and the tag builds the binaries, creates the GitHub release, and
publishes to crates.io. Merging only `ci:` or `docs:` commits changes nothing
about the version. Do not edit `CHANGELOG.md` or `version` yourself; the
release PR owns both.

`release-plz.toml` holds the mapping above. Two things live outside it:

- The `RELEASE_PLZ_TOKEN` repository secret: a fine-grained PAT with
  `Contents: write` and `Pull requests: write` on this repository. A tag or a
  pull request created with the default `GITHUB_TOKEN` starts no workflow, so
  without this token the tag would build no binaries and the release PR would
  run no CI.
- The release-plz version in `.github/workflows/release-plz.yml`, which
  Dependabot does not track and is bumped by hand.

`scripts/check-version.sh` checks that `Cargo.toml` holds a valid semantic
version without build metadata, that `CHANGELOG.md` has a dated section for it,
and, given a tag, that the tag agrees with both. CI runs it on every pull
request, and the release workflow runs it against the tag before building
anything:

```sh
sh scripts/check-version.sh v0.2.0
```

## License

By contributing, you agree that your contribution is dual-licensed under MIT OR Apache-2.0, unless you state otherwise.
