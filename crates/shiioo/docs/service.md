# Running as a service

`shiioo install` turns `shiioo serve` into something that starts at login,
restarts when it dies and keeps the machine awake. The office's data stays
in the workspace; the service definition lives in the user's home.

## What `install` does

1. Resolves the workspace to an absolute path and the binary to
   `current_exe()`, so moving the terminal's directory later changes nothing.
2. Captures the current `PATH` into the unit. Harnesses (`claude`, `codex`,
   `gh`, `git`) are found through it, and a login-item environment does not
   have the shell's `PATH`.
3. Stores the Telegram token from `--telegram-token` or
   `$SHIIOO_TELEGRAM_TOKEN` in `.shiioo/telegram-token` (mode 0600), if
   given.
4. Writes and loads the unit for the platform (below). With `--print` it
   only prints the unit.
5. Prints where the logs are, `dashboard on this machine:
   http://<listen>/#token=<token>`, and, if `tailscale status --json`
   answers, the Tailscale hints.

`shiioo uninstall` unloads and removes the unit and leaves `.shiioo/` in
place.

## macOS: launchd

Label `io.raskell.shiioo`, file
`~/Library/LaunchAgents/io.raskell.shiioo.plist`:

| Key | Value |
| --- | --- |
| `ProgramArguments` | `<exe> --workspace <ws> serve --listen <addr> --keep-awake` |
| `EnvironmentVariables` | `PATH` (captured), `HOME` |
| `WorkingDirectory` | the workspace |
| `RunAtLoad` | true |
| `KeepAlive` | true (relaunch whenever it exits) |
| `ProcessType` | `Standard` |
| `StandardOutPath`, `StandardErrorPath` | `<ws>/.shiioo/logs/serve.log` |

Loading: `launchctl bootout gui/<uid>/io.raskell.shiioo` (errors ignored, it
may not be loaded) then `launchctl bootstrap gui/<uid> <plist>`. Uninstall
runs the `bootout` and deletes the plist.

Useful commands:

```sh
launchctl print gui/$(id -u)/io.raskell.shiioo | head      # is it running?
tail -f <workspace>/.shiioo/logs/serve.log
launchctl kickstart -k gui/$(id -u)/io.raskell.shiioo      # restart
```

### Keep-awake

`--keep-awake` runs `caffeinate -s -i -w <pid of shiioo>`: no system sleep,
no idle sleep, for as long as the office process lives. The child is spawned
with `kill_on_drop`, so a crash of `serve` ends it too. Display sleep is not
prevented. It is macOS only; on other platforms the flag is accepted and does
nothing.

## Linux: systemd --user

File `~/.config/systemd/user/shiioo.service`:

```ini
[Unit]
Description=Shiioo office
After=network-online.target

[Service]
ExecStart=<exe> --workspace <ws> serve --listen <addr>
Restart=always
RestartSec=3
Environment=PATH=<captured>
WorkingDirectory=<ws>

[Install]
WantedBy=default.target
```

Loaded with `systemctl --user daemon-reload` and `systemctl --user enable
--now shiioo`; uninstall runs `disable --now` and deletes the file. Logs go
to the journal (`journalctl --user -u shiioo -f`). The Linux unit does not
pass `--keep-awake`, and it has not been exercised on a real machine yet.
A user service stops at logout unless `loginctl enable-linger` is set.

## Reaching it from the phone

The server listens on loopback by default because the API runs tools on this
machine. The intended path to the phone is Tailscale:

```sh
tailscale serve --bg 7878
```

exposes the dashboard as `https://<machine>.<tailnet>.ts.net` inside the
tailnet with a valid certificate. `install` prints that URL when it can read
`.Self.DNSName` from `tailscale status --json`, together with the reminder to
set `[dashboard] url` in `config.toml` so Telegram links point there.
Telegram itself needs no inbound connectivity: the bot long-polls.

Binding to a Tailscale IP with `--listen 100.x.y.z:7878` works too, but then
the hook endpoint the harnesses call is built from that address, so it must
be reachable from the machine itself.

## Operational notes

- **Restart cost.** Runs survive restarts: a worker that comes back reads
  the log and continues or resumes the harness session. Pending approvals
  stay visible but the parked hook request is lost; the harness sees a
  denial after its own timeout (see
  [approvals-and-capacity.md](../../shiioo-api/docs/approvals-and-capacity.md)).
- **Secrets on disk.** `.shiioo/api-token` and `.shiioo/telegram-token` are
  mode 0600; `serve` logs only the path of the token file, never the token.
  `install` prints the full dashboard link to the terminal once.
- **Upgrading.** Replace the binary, then `launchctl kickstart -k` or
  `systemctl --user restart shiioo`. The unit points at the binary path
  captured at install time, so reinstall if that path changed.
- **Several offices.** Each workspace is independent; install one service
  per workspace by changing the label is *(not supported yet)*, the label is
  fixed.

See also: [deployment design](../../../docs/design/deployment.md),
[security design](../../../docs/design/security.md),
[ADR-0010 single binary](../../../docs/adr/0010-single-binary-server-and-nodes.md).
