---
name: tmux-run
description: Run long-running, interactive, streaming, or uncertain-duration shell commands in detached tmux sessions with the tmux-run helper, which logs combined output to a file, appends an exit-status marker, and waits for that marker on request. Use when a command should keep running outside the tool call, when waiting on it directly could time out or block, or when its exit status must outlive the session as a file.
---

# tmux-run

`tmux-run` starts a command in a detached tmux session, writes its combined stdout and stderr to a log file, and appends `__DONE__:<status>` to the log when the command exits. Starting returns immediately; `tmux-run wait` later blocks on the same session and exits with the recorded status. It needs `tmux` and `bash` on `PATH`.

Use it for long-running, interactive, streaming, or uncertain-duration commands where a direct `bash` tool call is likely to time out or block. Do not use it for fast commands such as `pwd`, `ls`, a tight `find` or `rg`, `git status`, or ordinary validation expected to finish promptly.

## Start a task

```sh
tmux-run <task-name> -- <command> [args...]
```

Examples:

```sh
tmux-run build -- cargo test
tmux-run dev-server -- npm run dev
tmux-run smoke -- bash -lc 'set -euo pipefail; cd ~/src/app && ./scripts/smoke.sh'
```

On success it prints:

```text
session: <session-name>
log: <log-path>
wait: tmux-run wait '<session-name>'
completion marker: __DONE__:<status>
attach: tmux attach -t '<session-name>'
follow log: tail -f '<log-path>'
```

Report the session name, log path, and `tmux-run wait` command to the user when starting a background task.

- The session name is the task name with every character other than ASCII letters, digits, `-`, and `_` replaced by `_`, followed by the process id and a timestamp, so every run gets a new session. Use the printed name; never reconstruct it.
- The log is `output.log` in a `<session-name>` directory under `tmux-run/` in `$XDG_STATE_HOME`, then `~/.local/state`, then the system temp directory, next to the generated `run.sh`. tmux-run never deletes either.
- A usage error (no task name, no `--`, or nothing after it; no session name, a session name that is not one tmux-run printed, or a bad `--timeout`) prints `error: ...` and exits 2 without starting or waiting on anything.

## Monitor and finish

Check progress by reading the printed log path, for example:

```sh
tail -n 80 <log-path>
```

The log appears a moment after `tmux-run` returns, once tmux has started the task; retry a read that finds no file.

To block until the task finishes, wait on the session instead of polling:

```sh
tmux-run wait <session-name> --timeout 600
```

`tmux-run wait` returns as soon as the marker lands and exits with the recorded status. `--timeout` bounds the wait and exits `124`; a session that ended without a marker exits `3`. Always pass an explicit `--timeout`. Never run `tail -f` on the log from a tool call; it does not return.

To inventory tasks and their states, use:

```sh
tmux-run list
```

It prints one tab-separated line per task: session name, state (`running`, `done <n>`, or `ended`), and log path. `--json` prints the same as JSON objects with the recorded status where available.

To read a task's tail and status without blocking or needing the log path, use:

```sh
tmux-run show <session-name>
```

It prints the session name, its state, the log path, and the last 40 log lines. `--lines N` changes the count; `--json` prints the same as JSON. `show` exits 3 when there is no log for that session name.

Without waiting, a finished task's last log line is `__DONE__:<status>`, the command's exit status. Until it appears, the command is still running, or its session was killed before the command could exit, in which case no marker is written. Check the session directly:

```sh
tmux has-session -t "=<session-name>" 2>/dev/null && echo active || echo ended
```

If a completed session is still present, close only that session:

```sh
tmux kill-session -t <session-name>
```

Do not leave idle shells, completed sessions, or `tail -f` panes running.

## Inside Herdr

When `HERDR_ENV=1`, the agent is running inside [Herdr](https://herdr.dev), and the herdr skill's pane workflow is preferred for background commands: split a pane, `herdr pane run`, `herdr pane wait-output`, `herdr pane read`, then `herdr pane close`. Those runs stay visible in the user's layout and use stable pane IDs.

Use tmux-run instead when the result must outlive the session as a file. A Herdr pane keeps no combined log on disk, and `tmux-run wait` reports the command's exact exit status. tmux-run also works where no Herdr server exists, such as cron, plain SSH, and CI.

## Safety

- Put the command and its arguments after `--`. tmux-run writes them into a script as a quoted array, so argument boundaries survive and the task's shell does not re-parse them.
- Shell syntax on the tmux-run command line belongs to the calling shell: a pipe or redirect there applies to tmux-run's own output, not to the task. When the task needs pipes, redirects, or `&&`, wrap it in `bash -lc '...'`. Otherwise prefer direct arguments over nested quoting.
- When tmux-run starts the tmux server itself, the task inherits the calling shell's working directory and environment. An already running server may not pass exported variables on, so pass the ones the task needs explicitly, as `env NAME=value <command>` or inside `bash -lc '...'`.
- Never print secret values into commands or logs; the log stays on disk. Check a secret's presence only as `<set>` or `<unset>`.

## Updating

The skill text is embedded in the binary. To refresh this file from the installed `tmux-run`, run:

```sh
tmux-run skill --check <this skill's directory>
```

and, if it reports missing or stale, run:

```sh
tmux-run skill --install <this skill's directory>
```
