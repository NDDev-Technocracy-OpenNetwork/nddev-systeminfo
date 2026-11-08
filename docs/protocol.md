# NDDev Systeminfo protocol

The daemon publishes newline-delimited JSON over a per-user local socket.
Every client sees the same stream; every sample carries every field, with
`null` wherever the host cannot supply a reading — a client never has to
distinguish "missing" from "unsupported".

## Transport

| Platform | Socket |
|---|---|
| Linux | `$XDG_RUNTIME_DIR/rldyour-sysinfo.sock` (normally `/run/user/<uid>/rldyour-sysinfo.sock`) |
| macOS | `~/Library/Application Support/rldyour-sysinfo/rldyour-sysinfo.sock` |
| Windows | `%LOCALAPPDATA%\rldyour-sysinfo\rldyour-sysinfo.sock` |

The socket is mode `0600`: only the owning user may connect. On Linux the
socket is normally bound by systemd (`SocketMode=0600`) and on macOS by
launchd (`SockPathMode` 384), so the daemon only binds it itself when run
without a service manager — and applies the same mode either way.

## Handshake

On connect a client may write one line stating the cadence it wants, in
whole seconds:

```json
{"interval":5}
```

- Bounds are 0–60. `0` is not "never": it selects realtime, a 500 ms tick —
  the fastest cadence at which the readers stay meaningful. Anything outside
  the range is ignored.
- The daemon serves the fastest cadence any connected client requested and
  falls back to `sampling.interval_seconds` in the typed TOML configuration.
- A new connection wakes the daemon's loop at once and receives the latest
  completed snapshot. If that snapshot is already due at the requested
  cadence, the collector refreshes it first. Connecting does not force extra
  hardware reads or extra publications to existing clients.
- A client that sends nothing gets the configured cadence — the handshake
  is optional.
- The daemon reads at most 256 bytes and waits at most 250 ms total for the
  whole line. An oversized opening line is disconnected. Only a complete
  object with a whole-number `interval` is a cadence request; malformed or
  unsupported requests select the configured cadence.

## Sample

One JSON object per tick, terminated by `\n`:

```json
{"v":1,"cpu":{"usage":55.4,"temp":88.6},"memory":{"used":54.7,"swap":4.2},
 "gpu":{"usage":7.0,"memory":9.6,"temp":69.0},
 "disk":{"read":10628,"write":3789480,"temp":47.8},
 "net":{"rx":18360,"tx":342578}}
```

| Field | Type | Meaning |
|---|---|---|
| `v` | int | protocol version |
| `cpu.usage` | float % | busy share of all cores in the interval |
| `cpu.temp` | float °C | package temperature |
| `memory.used` | float % | physical memory in use |
| `memory.swap` | float % | swap in use; `null` when there is no swap |
| `gpu.usage` | float % | GPU busy share in the interval |
| `gpu.memory` | float % | video memory in use |
| `gpu.temp` | float °C | GPU temperature |
| `disk.read` / `disk.write` | int | bytes per second, physical devices |
| `disk.temp` | float °C | storage temperature |
| `net.rx` / `net.tx` | int | bytes per second, physical interfaces |

Rate fields need two samples to exist, so the first tick of a daemon that
cold-started reports them as `null`. Clients should check `v` and ignore a
version they do not implement; new fields may appear inside version 1
without a version bump — only an incompatible change increments it.

## Lifecycle

The server admits at most 32 clients. A client that cannot receive a complete
sample without blocking is disconnected and may reconnect on a backoff.
Every client should enforce the 4096-byte frame ceiling. When there are no
clients, the daemon does not collect hardware samples.

After `sampling.idle_release_seconds` without clients, it also drops the
collector and releases cached handles and optional driver resources. GPU
collection is opt-in through `sampling.gpu`. These settings do not change the
v1 endpoint identity or nullable metric shape.

Linux and macOS use socket activation (systemd user socket, launchd
`Sockets`), so the daemon may exit after thirty seconds without clients and
is respawned by the next connection. Windows has no per-user socket
activation; the daemon starts at login through the Run key and stays
resident. Either way a client that finds no listener should reconnect on a
short backoff rather than treat it as an error.
