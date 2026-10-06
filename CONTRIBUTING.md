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

## License

By contributing, you agree that your contribution is dual-licensed under MIT OR Apache-2.0, unless you state otherwise.
