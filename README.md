# tmux-run

[![CI](https://github.com/mhalder/tmux-run/actions/workflows/ci.yml/badge.svg)](https://github.com/mhalder/tmux-run/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/tmux-run)](https://crates.io/crates/tmux-run)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](https://github.com/mhalder/tmux-run#license)

`tmux-run` is a small, dependency-free wrapper around the usual hand-written tmux-and-logging boilerplate.

- Starts a command in a detached tmux session.
- Writes combined stdout and stderr to a log file.
- Appends a `__DONE__:<status>` completion marker when the command exits.
- Starting returns immediately; `tmux-run wait` blocks on the same session later and exits with the recorded status.

It optimizes for predictable behaviour rather than session management.

## Requirements

- `tmux` installed and on `PATH`
- `bash` (the command runs under bash)
- a Unix-like system

## Install

```sh
cargo install tmux-run
```

Prebuilt binaries are published on [GitHub Releases](https://github.com/mhalder/tmux-run/releases) for:

- Linux x86_64 and aarch64, static musl builds
- macOS x86_64 and aarch64

## Usage

```text
tmux-run <task-name> -- <command> [args...]
tmux-run wait <session-name> [--timeout <seconds>]
tmux-run list [--json]
tmux-run show <session-name> [--lines N] [--json]
tmux-run skill [--install [DIR]] [--check [DIR]]
tmux-run completion <bash|zsh|fish> [--install] [--check]
tmux-run --help | -h
tmux-run --version | -V
```

Start a task, then wait for it:

```sh
tmux-run build -- cargo test
tmux-run wait build_1234-5678 --timeout 600
```

`--` separates tmux-run's own arguments from the command and its arguments. The command runs under `bash`; stdout and stderr are combined into the log.

## Wait for completion

`tmux-run wait` blocks until the log records the completion marker, then exits with the recorded status. `--timeout <seconds>` bounds the wait.

| Exit code | Meaning |
| --------- | ------- |
| 0 | ok |
| 1 | a `--check` found the file missing or stale |
| 2 | usage error |
| 3 | `wait` found the session ended with no marker; `show` found no log for the session |
| 124 | `wait` timed out |

When `wait` finds the marker it exits with the command's recorded exit status rather than one of the codes above.

## List and show

`tmux-run list` inventories every task with a log, sorted by session name: running now, done with a recorded status, or ended without a marker. Each line is tab-separated:

```text
<session-name>	<state>	<log-path>
```

`<state>` is `running`, `done <n>`, or `ended`: a task is `done <n>` when the log's `__DONE__:<n>` marker is present, `running` when its tmux session is alive, and `ended` otherwise. `--json` prints a JSON array of `{"session","state","status","log"}` objects; `status` is the recorded exit status for `done`, otherwise `null`.

`tmux-run show <session-name>` prints the tail of a task's log without blocking, plus its state. `--lines N` defaults to 40.

```text
session: <session-name>
status: <state>
log: <log-path>

<last N log lines>
```

`--json` prints `{"session","state","status","log","lines"}`, where `lines` is an array of strings.

Both commands are read-only. `list` exits 0 (including when there are no tasks) and `show` exits 0 whenever the log exists; both exit 2 on a usage error.

## Output and state

`tmux-run` starts the session and exits immediately; it does not wait for the command to finish. On startup it prints:

```text
session: <session-name>
log: <log-path>
wait: tmux-run wait '<session-name>'
completion marker: __DONE__:<status>
attach: tmux attach -t '<session-name>'
follow log: tail -f '<log-path>'
```

The marker is appended to the log when the command exits, with the command's exit status.

The task name is normalized for tmux (every character outside ASCII letters, digits, `-`, and `_` becomes `_`) and combined with the process id and a timestamp to make the session name unique. State lives in `tmux-run/<session>/` under `$XDG_STATE_HOME`, then `~/.local/state`, then the system temp directory, holding `run.sh` and `output.log`. The command and its arguments are written into `run.sh` with argument boundaries preserved, rather than interpolated directly into the tmux command line. Nothing is deleted automatically.

## Agent skill

The skill text is embedded in the binary, so `tmux-run skill --install` writes it wherever a coding agent looks for skills and `tmux-run skill --check` verifies it. The canonical source in this repository is [`skills/tmux-run/SKILL.md`](skills/tmux-run/SKILL.md).

```sh
tmux-run skill --check
tmux-run skill --install
```

The default directory is `~/.agents/skills/tmux-run`. `--install` creates the directory and writes `SKILL.md`, reporting whether it was created, updated, or already up to date. `--check` byte-compares the file against the embedded copy and exits `1` when it is missing or stale. To point the skill at another agent's skills directory, pass the directory explicitly:

```sh
tmux-run skill --install /path/to/agent/skills/tmux-run
tmux-run skill --check /path/to/agent/skills/tmux-run
```

## Shell completion

`tmux-run completion <shell>` prints the completion script for `bash`, `zsh`, or `fish`. `--install` writes it to the per-shell path, and `--check` byte-compares the installed file against the embedded copy.

| Shell | Install path |
| ----- | ------------ |
| bash | `~/.local/share/bash-completion/completions/tmux-run` |
| zsh | `~/.local/share/zsh/site-functions/_tmux-run` |
| fish | `~/.config/fish/completions/tmux-run.fish` |

zsh users need the `site-functions` directory on `$fpath` for the completion to load:

```sh
fpath=(~/.local/share/zsh/site-functions $fpath)
```

## License

Licensed under MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). To report a vulnerability, see [SECURITY.md](SECURITY.md).
