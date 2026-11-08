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
| `gpu.usage` | float % | busiest available adapter's utilization; Windows uses its busiest engine |
| `gpu.memory` | float % | dedicated video memory in use when a dedicated total is available; UMA remains null |
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
collector and releases cached handles and optional driver resources. Passive GPU
collection is enabled in new configurations; `sampling.gpu=false` disables it.
Existing explicit preferences are preserved. Suspended adapters are not probed.
These settings do not change the
v1 endpoint identity or nullable metric shape.

Linux and macOS use socket activation (systemd user socket, launchd
`Sockets`), so the daemon may exit after thirty seconds without clients and
is respawned by the next connection. Windows has no per-user socket
activation; the daemon starts at login through the Run key and stays
resident. Either way a client that finds no listener should reconnect on a
short backoff rather than treat it as an error.

## Native analysis and history

Ordinary v1 clients retain only the latest sample. Native status interfaces
request rolling history without changing the 4096-byte v1 frame:

```json
{"interval":5,"record":true}
```

An analysis reader negotiates the detailed frames defined by
[`analysis-v1.schema.json`](../contracts/analysis-v1.schema.json):

```json
{"interval":5,"details":true,"history":true,"record":true}
```

The provider first replays available detailed frames, sends
`{"history_end":true}`, then streams new detailed frames. Sequence numbers
allow deduplication; they restart with the collector. `details` increases the
per-frame limit to 262144 bytes. `history` requires `details`. Consumers that
only implement the original protocol should keep their original handshake.

The added `detail` contains wall-clock `at_ms`, suspend-inclusive `elapsed_ms`,
`sequence`, expected `period_ms`, `gpu_enabled`, logical `cores` and per-adapter
`gpus`. Core IDs are OS logical processor IDs. The first usage reading after
initialization or a detected sleep/clock discontinuity is null. Adapter status
is `available`, `sleeping` or `unavailable`; missing measurements stay null.
Dedicated used/total and shared used memory are separate byte counters.
Apple unified memory never becomes a fabricated VRAM percentage.

One in-memory ring is bounded by one hour, 7201 records and 8 MiB of retained
record payload/ownership estimates. It contains measurements made while a
recording client was monitoring; no historical measurements are invented.
Large machines or rapid sampling may reach the byte ceiling before one hour.
Removing recording clients releases history on the next sample; idle release
frees the collector and ring. There is no history database or background disk
writer. Replays share immutable records and have an absolute write deadline;
slow readers cannot stop the sampler.

`nddev-systeminfo history` exports available detailed records once.
`nddev-systeminfo analyze --metric cpu.usage --range-seconds 3600 --once`
returns the same bounded plot/statistics model used by native windows; omit
`--once` for live updates. Series include all v1 field paths, `core:<id>` and
`gpu:<adapter-id>:usage|temperature|dedicated_used|shared_used`.
`nddev-systeminfo ui` opens the installed native interface.

The view is independently versioned (`v:1`) by
[`analysis-view-v1.schema.json`](../contracts/analysis-view-v1.schema.json).
`nddev-systeminfo schema --view` exports this schema; the Rust contract exposes
the generated view/point/statistics types and bounded `decode_view` validation.
Native interfaces reject an unsupported view version. This presentation
contract does not change the ordinary v1 snapshot or its SDK consumers.

Plot columns preserve observed minimum/maximum peaks. The horizontal axis
always covers the requested interval. Nulls, dropped sequences, clock resets
and missing intervals create gaps. Averages are weighted by valid measured
intervals, with sample count and covered duration reported separately. Pausing
or closing a window stops its reader; it does not stop other monitoring clients.
The CLI view frame limit is 1 MiB. The standalone core, v1 SDK and NDS adapter
remain independent of any GUI, account, NDS service or credential.
