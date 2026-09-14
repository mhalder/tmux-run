---
name: tmux-run
description: Run long-running, interactive, streaming, or uncertain-duration shell commands in detached tmux sessions with the tmux-run helper, which logs combined output to a file and appends an exit-status marker. Use when a command should keep running outside the tool call, or when waiting on it directly could time out or block.
---

# tmux-run

`tmux-run` starts a command in a detached tmux session, writes its combined stdout and stderr to a log file, and appends `__DONE__:<status>` to the log when the command exits. It returns immediately and never waits for the command. It needs `tmux` and `bash` on `PATH`.

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
completion marker: __DONE__:<status>
attach: tmux attach -t '<session-name>'
follow log: tail -f '<log-path>'
```

Report the session name, log path, and completion marker to the user when starting a background task.

- The session name is the task name with every character other than ASCII letters, digits, `-`, and `_` replaced by `_`, followed by the process id and a timestamp, so every run gets a new session. Use the printed name; never reconstruct it.
- The log is `output.log` in a `tmux-run-<session-name>` directory under the system temp directory, next to the generated `run.sh`. tmux-run never deletes either.
- A usage error (no task name, no `--`, or nothing after it) prints `error: ...` and exits 2 without starting anything.

## Monitor and finish

Check progress by reading the printed log path, for example:

```sh
tail -n 80 <log-path>
```

The log appears a moment after `tmux-run` returns, once tmux has started the task; retry a read that finds no file.

A finished task's last log line is `__DONE__:<status>`, the command's exit status. Until it appears, the command is still running, or its session was killed before the command could exit, in which case no marker is written. Never run `tail -f` on the log from a tool call; it does not return.

When the task is complete, collect the final status from the marker and verify the tmux session ended:

```sh
tmux has-session -t <session-name> 2>/dev/null && echo active || echo ended
```

If a completed session is still present, close only that session:

```sh
tmux kill-session -t <session-name>
```

Do not leave idle shells, completed sessions, or `tail -f` panes running.

## Safety

- Put the command and its arguments after `--`. tmux-run writes them into a script as a quoted array, so argument boundaries survive and the task's shell does not re-parse them.
- Shell syntax on the tmux-run command line belongs to the calling shell: a pipe or redirect there applies to tmux-run's own output, not to the task. When the task needs pipes, redirects, or `&&`, wrap it in `bash -lc '...'`. Otherwise prefer direct arguments over nested quoting.
- When tmux-run starts the tmux server itself, the task inherits the calling shell's working directory and environment. An already running server may not pass exported variables on, so pass the ones the task needs explicitly, as `env NAME=value <command>` or inside `bash -lc '...'`.
- Never print secret values into commands or logs; the log stays on disk. Check a secret's presence only as `<set>` or `<unset>`.
