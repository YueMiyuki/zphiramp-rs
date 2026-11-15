# zphiramp

`zphiramp` is the Phira multiplayer server of the ZENITH event, written in Rust. It replaces the
Java server `jphira-mp` (ZENITH fork) in place: same `data/` directory and `frontend/dist`, same
ports and flags, same protocol, HTTP API and messages for clients and the web manager, with the
Java server's logic flaws fixed.

## Build

Needs Rust 1.85 or later (edition 2024) and a C compiler (the TLS library builds C code). From the
repository root:

```sh
cargo build --release -p zphiramp
```

The binary is `target/release/zphiramp`, a single file. At run time the host needs its usual CA
certificates to reach the Phira API over HTTPS.

## Run

```sh
cd /opt/zphiramp && ./zphiramp --port 12346 --http-port 8080
```

Run it from the directory that holds the state, as with the Java jar:

```text
/opt/zphiramp/
├── frontend/dist/   the web manager's static build, served on the HTTP port
├── data/            all state, created on first start; keep it across upgrades
└── logs/            latest.log and dated .log.gz archives (never deleted; prune them yourself)
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--port` / `--host` | `12346` / `0.0.0.0` | Game port and its bind address. |
| `--http-port` / `--http-host` | `8080` / `0.0.0.0` | HTTP port (admin API and web manager) and its bind address. |
| `--proxy-protocol [true\|false]` | `false` | Expect a PROXY protocol header on the game port. Unlike Java, the bare flag enables it. |
| `--read-timeout <s>` | `3600` | Idle timeout after login, at least 600 s. Unlike Java, it no longer applies before login: every connection must log in within 20 seconds. |
| `--max-connections` | `1000` | Simultaneous connections (at most 64 per address). |
| `--max-rooms` | `200` | Rooms alive at once. |
| `--empty-room-ttl <min>` | `60` | Minutes before an empty operator-created room is reclaimed; `0` disables. |
| `--language` | `zh-CN` | Default player language. |
| `--max-pending-submissions <n>` | `50` | New. Pending chart submissions per account; `-1` for no limit (`0` or below `-1` keeps the default, with a warning). |
| `--login-rate-limit <n>` | `30` | New. HTTP login attempts per minute per client address; `-1` for no limit (likewise). |
| `--plugins` | `plugins` | Accepted and ignored, as in Java. |
| `--help`, `-h` | | Print the usage to standard output and exit with code 0. |

Write each flag in full as `--flag value` or `--flag=value`. Unlike Java, abbreviations (`--po`)
and single-dash long flags are not accepted, a repeated flag is an error, and so is any word that
is not a flag. An argument error is printed to standard error and exits with code 1; neither
`--help` nor an error creates `logs/` or `data/`.

The environment variable `ZPHIRA_PHIRA_HOST` overrides the Phira API address (default
`https://phira.5wyxi.com/`), for testing against a mock.

## Moving from the Java server

- **Carries over**: `data/` (including `jwt-secret`, so web manager logins stay valid),
  `frontend/dist`, ports, flags, game clients and the web manager.
- **First start**: chart durations stored by the Java server are cleared once, because Java
  measured some audio files wrongly, and are probed again when needed. The marker
  `data/.duration-probe-v2` records the reset; delete it to clear durations again.
- **Know before switching**:
  - A bare `--proxy-protocol` now turns proxy protocol on.
  - Clients have 20 seconds from connecting to log in.
  - SIGTERM (`systemctl stop`) stops the server cleanly within seconds.
  - A round in progress at stop is abandoned: no points, no round record.
  - Room hosts and room admins can clear their room (see below).
  - Files under `data/` are written to a temporary file and renamed, so a crash never leaves a
    half-written file. Imports are validated before anything is written, and only the newest 20
    backups per data kind are kept in `data/backups`.
  - Data files must be strict JSON: a hand edit with comments, single quotes, a number written
    as a string or a repeated key cannot be read (Java tolerated them). Timestamps are written with
    seconds; the Java form without seconds is still read.
  - Command lines must spell flags out in full (see above).
- **Going back** works: the Java server reads the files `zphiramp` writes. Unlock locked rooms
  first (the Java server cannot unlock them), and delete `data/.duration-probe-v2` before returning to
  `zphiramp` if the Java server probed durations in between.

## Deployment

`/etc/systemd/system/zphiramp.service`:

```ini
[Unit]
Description=zphiramp (Phira multiplayer server)
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/zphiramp
Environment=GAME_PORT=12346
Environment=HTTP_PORT=8080
# 0.0.0.0: the web manager is reachable directly at http://<server>:8080
# 127.0.0.1: only a reverse proxy on this host can reach it
Environment=HTTP_HOST=0.0.0.0
ExecStart=/opt/zphiramp/zphiramp --port ${GAME_PORT} --http-port ${HTTP_PORT} --http-host ${HTTP_HOST}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

`WorkingDirectory` must hold `data/` and `frontend/dist`. Exit codes: 0 after `stop`, SIGTERM or
SIGINT; 1 after an argument error, a failed start, or an internal failure (it stops cleanly
first, so systemd restarts it).

Change the three `Environment=` lines (or override them with `systemctl edit zphiramp`) to move
the ports or the HTTP bind address. With `HTTP_HOST=0.0.0.0`, the server's default, the web
manager is reachable directly, as with the Java server.

The HTTP port speaks plain HTTP and the login carries Phira passwords. To serve it over HTTPS,
put a TLS reverse proxy in front and set `HTTP_HOST=127.0.0.1` so that only the proxy can reach
it. Behind a proxy all users share one login limit, so raise `--login-rate-limit` if needed.

## Console

Type commands on standard input (a line editor with history on a terminal; Ctrl-C is `stop`).

| Command | Purpose |
| --- | --- |
| `stop` | Stop the server. |
| `say <text>` | Message every online player. |
| `admin list` / `admin add <userId>` / `admin remove <userId>` | Admin list. |
| `room list` / `room create <id> [poolId...]` / `room remove <id>` | Rooms. |
| `room <id> end` | Force-end the current round. |
| `room <id> clear` | Remove and disconnect everyone in the room. |
| `room <id> config <key> <value>` | `minPlayer`, `maxPlayer`, `selectCountdown`, `readyCountdown`, `forceFinish`, `interval`, `cancelReady`, `startAllReady`, `allReadyDelay` (keys ignore case). |
| `room <id> pool switch <poolId>` / `room <id> pool favorite <id\|none>` | Pool for the next round; favorite override. |
| `pool list` / `pool add <id> [chartIds...]` / `pool remove <id>` | Pools (a pool may start empty). |
| `pool chart add\|remove <poolId> <chartId>` | Charts of a pool. |
| `pool favorite <poolId> <id\|none>` / `pool default <poolId> <on\|off>` | Favorite collection; used by players' rooms by default. |
| `pool meta <poolId> <category> [sizeLimit] [roundsPerStay] [order]` | Category and rotation. |
| `pool generate <category> [sizeLimit] [roundsPerStay] [probeDuration]` | Cut matching charts into new pools; `probeDuration` is the duration-probe budget. |
| `chart index` / `chart search <category> [limit]` / `chart duration <chartIds...>` | Index status; screening; duration probe. |
| `chart refresh [plain\|full]` | Update the Phira catalogue in the background; `full` walks the whole listing. |
| `submission list [poolId]` | Pending submissions, or all of one pool. |
| `submission approve <poolId> <chartId>` / `submission reject <poolId> <chartId> [reason]` | Review a submission. |

**Ready settings.** Each room has three settings, set in the web manager's room dialog or with
`room <id> config` (console keys in parentheses, on/off values as `on`/`off`, `true`/`false` or
`1`/`0`), and saved in `rooms.json`: `cancelReadyInVote` (`cancelReady`: a ready pressed during
voting may be cancelled; default on), `startWhenAllReady` (`startAllReady`: start before the ready
countdown ends once every online player is ready; default on; off means no early start at all)
and `allReadyDelay` (seconds to wait first, 0 to 300; default 5; `0` starts at once). The room is
told, for example, `全员已准备，5 秒后开始游戏。`; if someone cancels ready or a player who is not
ready joins or comes back, the start is called off (`有玩家尚未准备，开始已取消。`). Spectators never
count towards "everyone is ready", and every online spectator watches the round whether or not it
pressed ready (Java left out spectators that had not pressed ready); this has not been tried with
a real game client.

**Clearing a room.** The room's host (the first player who joined and is still in it) or one of
its room admins can send `/clear` in room chat. It is never shown as chat and works with chat
off; anyone else gets `你不是房主`. Everyone else in the room, players and spectators, is
removed, including players who are temporarily disconnected; those online receive
`房间已被清空，你已被移出房间。` and are disconnected. A round in progress ends. Global admins and the room's controllers can do the same
over HTTP (`POST /api/v1/room/{id}/clear`, answering `{"ok":true,"removed":n}`), and the operator
with `room <id> clear`; these remove everyone. A room players created in game closes once empty;
rooms created by an operator stay.

## Web manager

The existing web manager works unchanged. A patched copy of its repository adds controls for the
new features (room clearing, room lock, ready settings, full catalogue refresh).

## Tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

## Licence

GNU Lesser General Public License v3, like the Java server it is derived from (Copyright (C) 2026
lRENyaaa). See `LICENSE`.
