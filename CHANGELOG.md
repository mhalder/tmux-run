# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1](https://github.com/mhalder/tmux-run/compare/v0.3.0...v0.3.1) - 2026-10-09

### Fixed

- state cleanup, marker robustness, and log tail window ([#12](https://github.com/mhalder/tmux-run/pull/12))

## [0.3.0](https://github.com/mhalder/tmux-run/compare/v0.2.0...v0.3.0) - 2026-10-07

### Added

- add clean and rm subcommands for task state

## [0.2.0](https://github.com/mhalder/tmux-run/compare/v0.1.1...v0.2.0) - 2026-10-07

### Added

- add list and show subcommands ([#6](https://github.com/mhalder/tmux-run/pull/6))

## [0.1.1](https://github.com/mhalder/tmux-run/compare/v0.1.0...v0.1.1) - 2026-10-06

### Fixed

- accept --help for the skill and completion subcommands ([#3](https://github.com/mhalder/tmux-run/pull/3))

## [0.1.0] - 2026-10-06

### Added

- Start a command in a detached tmux session with `tmux-run <task-name> -- <command>`, logging combined stdout and stderr and appending a `__DONE__:<status>` completion marker.
- Wait for a session to finish with `tmux-run wait`, including `--timeout`.
- The embedded agent skill, exposed via `tmux-run skill` with `--install` and `--check`.
- Shell completion for bash, zsh, and fish via `tmux-run completion` with `--install` and `--check`.

### Fixed

- Shell quoting doubled the UTF-8 encoding of non-ASCII arguments.
- A command whose output did not end with a newline concatenated with the completion marker, so `tmux-run wait` reported exit 3 instead of the recorded status.
