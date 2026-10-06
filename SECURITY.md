# Security

## Supported versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |

## Reporting a vulnerability

Report vulnerabilities through GitHub private vulnerability reporting:

https://github.com/mhalder/tmux-run/security/advisories/new

Do not open a public issue for a security report.

## What matters here

tmux-run is a small wrapper, so the security-relevant surface is narrow:

- Shell quoting and argument handling: command arguments are written into a generated script and must not be re-parsed or allow command injection.
- Path handling in the state directory: session names are validated before use, and state paths are built from that validated input.
- Anything that could run an unintended command.

Logs stay on disk under the state directory; treat log contents as sensitive.
