# NDDev Systeminfo

[![CI](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/actions/workflows/ci.yml/badge.svg)](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/actions/workflows/ci.yml)

Low-overhead live system metrics from one Rust daemon on Linux, macOS and
Windows. Linux has a GNOME Shell indicator, macOS has a native menu bar client,
and every platform exposes the same versioned local JSON protocol.

The protocol also has a dependency-free Python client published as
`rldyour-sysinfo` on PyPI. The Rust daemon is published as
`rldyour-sysinfod` on crates.io.

Current stable release: **0.3.1**.

![The indicator in the top bar](docs/panel.png)

## Why it is two pieces

Platform UI processes stay deliberately thin. Collection, rate calculation and
hardware access live in the Rust daemon; clients only decode one JSON line and
render it. Linux and macOS use Unix domain sockets, while Windows uses its
native AF_UNIX implementation available since Windows 10.

The split is not only a workaround. Under Wayland the shell cannot be restarted
without logging out, so any change to extension code costs a session. Keeping
the extension thin and stable, and putting everything that evolves into a
daemon that restarts in milliseconds, is what makes the thing maintainable.

### What the design deliberately avoids

- **No async runtime in Rust.** The daemon owns one sampling thread and one
  accept thread. Native hardware libraries can also create their own threads.
- **No D-Bus stack.** `zbus` spawns a thread per connection and its own
  executor. For pushing under two hundred bytes a tick, a plain `UnixListener`
  from the standard library costs nothing and depends on nothing.
- **No process scanning.** Linux reads `/proc` and `/sys` directly. macOS uses
  Mach, BSD, IOKit, AppleSMC and IOHID. Windows refreshes system, component and network
  counters without constructing a process list.
- **No polling in the shell.** The cadence belongs to the daemon; the extension
  only waits on the next line. It runs no timer while connected, which also
  removes the most common cause of leaks in shell extensions.

## Cost

The server admits at most 32 clients, keeps at most 32 accepted connections
queued, reads at most 256 handshake bytes and waits at most 250 ms total.
Writes are nonblocking: a reader with a full buffer is disconnected. One
shared sampling clock caps hardware reads at twice a second even during
connection churn. When no clients are attached, no samples are collected.
The JSON buffer is reused; native APIs may allocate temporary objects.

The macOS client uses dispatch socket readiness instead of a blocked reader
thread, and every client bounds incoming frames to 4096 bytes.

Historical Linux measurements on the development machine, resident private
memory (`Pss`):

| Configuration | Pss | Threads | Binary |
|---|---|---|---|
| Daemon without GPU metrics | ~0.6 MB | 2 | 375 KB |
| Daemon with NVIDIA metrics | ~19.6 MB | 3 | 484 KB |

The entire difference is NVML. The proprietary NVIDIA driver publishes no
utilisation or temperature through `procfs` or `sysfs` and binds no hwmon node,
so NVML is the only interface that answers, and it maps about nineteen
megabytes of driver-side state. Set `RLDYOUR_SYSINFO_GPU=0` in the service
environment, or build with `--no-default-features`, and that cost disappears
along with the GPU readings.

## Platform support

| Metric | Linux | macOS | Windows |
|---|---|---|---|
| CPU load | `/proc/stat` | Mach host statistics | Windows system counters |
| CPU temperature | hwmon | AppleSMC / IOHID | hardware component provider |
| Memory and swap | `/proc/meminfo` | Mach VM statistics | Windows memory counters |
| GPU load, memory, temperature | NVIDIA NVML | IOAccelerator and AppleSMC / IOHID | NVIDIA NVML |
| Disk throughput | `/proc/diskstats` | IOKit storage statistics | Windows volume counters |
| Network throughput | `/proc/net/dev` | BSD interface counters | Windows network counters |

Throughput counts bytes per second on interfaces and devices the OS considers
physical: Linux follows real `/sys` device nodes, macOS counts up `en*`
interfaces, and Windows excludes the loopback, tunnel, VPN, and hypervisor
adapters it can identify by name.

Unsupported or unavailable hardware readings are always `null`. The daemon
does not substitute estimates. NVIDIA metrics can be disabled at build time
with `--no-default-features`.

## Install on Linux

Requires Rust 1.95 or newer and GNOME Shell 46 (Ubuntu 24.04 LTS) through 50.

Ubuntu amd64 can install the signed package repository:

```sh
curl -fsSL https://nddev-technocracy-opennetwork.github.io/nddev-systeminfo/apt/rldyour-sysinfo.asc \
  | sudo tee /usr/share/keyrings/rldyour-sysinfo.asc >/dev/null
echo "deb [arch=amd64 signed-by=/usr/share/keyrings/rldyour-sysinfo.asc] https://nddev-technocracy-opennetwork.github.io/nddev-systeminfo/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/rldyour-sysinfo.list
sudo apt update
sudo apt install rldyour-sysinfo
```

The package installs the Rust daemon and its socket-activated user service.
Install the GNOME indicator from the matching GitHub release asset.

```sh
./install.sh
```

This builds the daemon into `~/.local/bin`, installs a socket-activated user
service, and copies the extension into place. Nothing needs root.

The daemon starts on the first connection and stops again thirty seconds after
the last client disconnects, so it consumes nothing while the extension is off.

Because Wayland cannot restart the shell in place, log out and back in, then:

```sh
gnome-extensions enable rldyour-sysinfo@nddev-opennetwork
```

To remove everything: `./uninstall.sh`.

## Install on macOS

Requires macOS 12 or newer, Rust 1.95 or newer and the Swift 6 toolchain shipped
with Xcode Command Line Tools.

```sh
./install-macos.sh
```

The installer builds the same Rust daemon with native Mach, BSD, IOKit and SMC
readers, creates a small native menu bar app, and registers both with per-user
LaunchAgents. launchd owns the socket and starts the daemon on the first
connection — the same socket-activation model systemd provides on Linux — so
it exits when nobody is watching. It does not install or invoke a third-party
monitor and needs no administrator privileges. Apple GPU load comes from
IOAccelerator performance statistics; CPU and GPU temperatures come from
read-only AppleSMC sensors with an IOHID temperature fallback on supported
Apple silicon models.

Remove it with `./uninstall-macos.sh`.

## Install on Windows

Requires Windows 10 or newer and Rust 1.95 or newer. Run PowerShell as the
current desktop user:

```powershell
.\install-windows.ps1
```

This installs the daemon under `%LOCALAPPDATA%\rldyour-sysinfo` and registers
it in the `Run` key, so it starts at login with no console window. Remove it
with `.\uninstall-windows.ps1`. The Windows daemon publishes the same
protocol; a native tray client is not part of the 0.2 series.

## Configuration

Open the extension's preferences for what most people want to change:

- **Interval** — a cadence preset: Economy (10 s), Standard (5 s), Fast (2 s)
  or Realtime, which puts the daemon on half-second ticks. The indicator asks
  the daemon for this cadence when it connects, and the daemon serves the
  fastest any connected client requested. Any whole number from 0 (realtime)
  to 60 seconds can also be set directly on the `interval` key.
- **Panel** — which of processor, memory, graphics and network appear in the
  top bar, and whether temperatures are shown beside them. The dropdown always
  lists everything regardless.

The daemon accepts these environment variables. On Linux they can be placed in
an `[Service]` drop-in under `~/.config/systemd/user/rldyour-sysinfod.service.d/`;
on macOS, add them to the
daemon LaunchAgent's `EnvironmentVariables` dictionary.

| Variable | Default | Meaning |
|---|---|---|
| `RLDYOUR_SYSINFO_INTERVAL` | `5` | Cadence when no client asks for one; `0` selects realtime (500 ms) |
| `RLDYOUR_SYSINFO_GPU` | unset | Set to `0` to skip NVML entirely |

## Protocol

A client may open with a single line stating the cadence it wants — `0` is
realtime (500 ms ticks), `1`–`60` a whole number of seconds — which the daemon
honours and otherwise ignores:

```json
{"interval":5}
```

The daemon then sends one newline-terminated JSON object per tick. Every metric is always present;
one the host cannot supply is `null`, so a client never distinguishes "missing"
from "unsupported". `v` is incremented only on an incompatible change.

```json
{"v":1,"cpu":{"usage":55.4,"temp":88.6},"memory":{"used":54.7,"swap":4.2},
 "gpu":{"usage":7.0,"memory":9.6,"temp":69.0},
 "disk":{"read":10628,"write":3789480,"temp":47.8},
 "net":{"rx":18360,"tx":342578}}
```

Percentages are per cent, temperatures are degrees Celsius, disk and network
figures are bytes per second. [`docs/protocol.md`](docs/protocol.md) is the
full specification every client implements — transport locations, handshake,
field meanings, and lifecycle.

Python clients can use `pip install rldyour-sysinfo`; the command
`rldyour-sysinfo --once` prints a single live sample from the local daemon.

## Checks

```sh
./scripts/check-extension.sh                      # what CI runs for the extension
cd daemon && cargo test && cargo clippy --all-targets --all-features -- -D warnings
cd extension && gjs -m tests/smoke.js             # needs the daemon reachable
swiftc -swift-version 6 -parse-as-library -typecheck macos/*.swift       # macOS menu client
```

`scripts/check-extension.sh` covers syntax, metadata, the settings keys the code
actually reads, process isolation between the shell and preferences processes,
and deprecated modules. It needs no running shell, which matters because
Wayland gives no way to reload extension code without a new login.

[`docs/quality.md`](docs/quality.md) records the resource limits, module
boundaries, platform checks and primary references behind the implementation.

## Licence

AGPL-3.0-or-later.

Low-level macOS API precedents and dependency licences are recorded in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

Note that this places the extension outside what extensions.gnome.org accepts:
the portal requires every extension to be distributable under GPL-2.0-or-later,
which AGPL-3.0 is not compatible with. The portal separately forbids shipping
binaries, which this project needs. Distribution is therefore from this
repository.
