# NDDev Systeminfo

[![CI](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/actions/workflows/ci.yml/badge.svg?branch=dev)](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/actions/workflows/ci.yml)

Independent, read-only system information for Linux, macOS and Windows. One Rust
provider serves the CLI, native desktop interface and optional NDS consumers.
It works without an NDS installation, account or credentials.

| Platform | Interface | Collection |
| --- | --- | --- |
| Linux | GNOME Shell indicator and native preferences | procfs, sysfs, optional NVIDIA NVML |
| macOS | AppKit menu-bar application | Mach, BSD, read-only IOKit/SMC/HID |
| Windows | Native notification icon and menu | selective system counters, optional NVIDIA NVML |

The interface supports English, Russian, Kazakh, Arabic and Chinese, RTL, four
OpenNetwork themes and OS accessibility preferences. Native controls retain
platform appearance. Shared brand geometry, colors and metrics come from the
public Apache-2.0 [OpenNetwork design system](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-opennetwork-design-system).

CPU, memory, swap, GPU, temperatures, disk and network rates use the same
[versioned local contract](docs/protocol.md). Unavailable readings are `null`;
disconnected interfaces clear their numbers immediately.

## Install

Download the matching qualified archive and `SHA256SUMS` from
[Releases](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/releases).
Verify the archive against the checksum file, extract it and open its directory.
Archives contain `bin/`, native presentation, the integrity manifest and notices.

Linux and macOS:

```console
./bin/nddev-systeminfo install
```

Windows PowerShell:

```powershell
.\bin\nddev-systeminfo.exe install
```

Installation uses the current user's directories and the native startup manager.
Use `install --provider-only` when the service and CLI are the intended interface.
Linux's GNOME extension becomes available through the Extensions application;
an existing Wayland session can require the next login to discover new code.

Linux requires a systemd user manager; the desktop interface targets GNOME 46–50.
macOS requires 12 or later and supports Apple silicon and Intel. macOS archives
carry an ad-hoc signature; Developer ID signing and notarization are not configured.
Windows releases target x64 Windows 10 (1803 or later)/11, with the C runtime linked statically.
Windows registers a per-user maintenance package with Windows Installer; removal
can also be started from Installed apps. Its CLI hands removal to the OS and
exits before the cached cleanup action deletes the installed executables.

To remove the user installation, run its CLI:

```console
nddev-systeminfo uninstall
```

The user configuration is preserved. Installation and service operations preserve
explicit disablement. A complete release manifest is verified before copying files.

### APT

The signed repository is
`https://nddev-technocracy-opennetwork.github.io/nddev-systeminfo/apt`.
After installing its public key in `/usr/share/keyrings/rldyour-sysinfo.asc`, add:

```console
curl --fail --location https://nddev-technocracy-opennetwork.github.io/nddev-systeminfo/apt/rldyour-sysinfo.asc --output rldyour-sysinfo.asc
gpg --show-keys --fingerprint rldyour-sysinfo.asc
sudo install -m 644 rldyour-sysinfo.asc /usr/share/keyrings/rldyour-sysinfo.asc
```

The public key fingerprint is `3962 D52A 20C4 C41E 1F0F 79F7 1921 782F 1736 B4DF`.

```text
deb [arch=amd64 signed-by=/usr/share/keyrings/rldyour-sysinfo.asc] https://nddev-technocracy-opennetwork.github.io/nddev-systeminfo/apt stable main
```

Install `nddev-systeminfo` through APT. Existing `rldyour-sysinfo` installations
upgrade through a transitional package. APT owns the system installation;
the portable installer owns a user installation. Select one provider owner.
The portable installer refuses an existing system package or foreign service
owner. The public key, repository label and user-unit identities preserve the existing
APT and socket trust state during the product rename.

## Configure and use

All defaults live in [config/default.toml](config/default.toml). A user file is
complete, typed TOML; unknown, missing and out-of-range fields are rejected.
New configurations use passive GPU collection and a five-second cadence.
Existing explicit preferences are preserved. Suspended adapters are not probed;
hardware libraries are loaded only when needed.

```console
nddev-systeminfo config path
nddev-systeminfo config init
nddev-systeminfo config set display.locale ru
nddev-systeminfo config set sampling.interval_seconds 0
nddev-systeminfo sample
nddev-systeminfo watch
nddev-systeminfo schema
nddev-systeminfo sample --details
nddev-systeminfo analyze --metric core:0 --range-seconds 3600 --once
nddev-systeminfo ui
```

`0` selects realtime, capped at 500 ms. Field edits are atomic and serialized
across processes. UI preferences and CLI use the same file. Explicit legacy
values, including realtime and false toggles, migrate without replacing an
existing configuration.

Cadence and presentation changes reach connected interfaces. To change provider
collection, save the setting and apply it through the native manager:

```console
nddev-systeminfo config set sampling.gpu true
nddev-systeminfo service apply
nddev-systeminfo service status
```

`service` also supports `start`, `stop`, `enable`, `disable` and `restart`.
Configuration follows XDG on Linux, Application Support on macOS and LocalAppData
on Windows. Product settings use TOML; OS path and locale conventions remain native.

Click a metric cell to open native details. Processor details show every logical
core, with bounded pages on large hosts. GPU details distinguish disabled,
unavailable and sleeping adapters; shared memory is bytes, separately from
dedicated memory. The native analysis window selects any reading or core,
15/30/60-minute ranges, pause/resume and JSON copy. OpenNetwork's themes and
five locales also apply to these windows, with a direct website action.
The installed `ui` launcher opens the overview. Linux also accepts a metric
shortcut; macOS and Windows use the running app's profile and window controls.

History records up to one hour while a native monitoring client is connected,
bounded by 7201 records and 8 MiB of retained record estimates. The window shows
actual covered time; gaps and unsupported readings remain visible. History is
RAM-only and clears after idle release. Ordinary v1 integration clients keep
only a cached current sample unless they explicitly request recording. See
[the negotiated analysis contract](docs/protocol.md#native-analysis-and-history).

## Resource behavior

One demand-driven sampler serves all clients at the fastest requested cadence.
Frames, admission, buffers and deadlines are bounded. Without clients, sampling
stops and the collector releases its files, native handles and GPU library after
the configured idle period. Socket-activated Linux/macOS providers exit then;
the standalone/Windows listener blocks without a polling timer.

A release-build comparison on one Linux host used separate executable copies,
realtime demand, ten seconds of active measurement and ten seconds of idle CPU
measurement after resource release:

The following historical measurements use the 0.3.2 candidate; 0.3.3 and 0.3.4 correct distribution
and package lifecycle while keeping the same collector and resource policy.

| Provider profile | Active PSS | PSS after release | Threads |
| --- | --- | --- | --- |
| 0.3.1, GPU explicitly disabled | about 0.74 MiB | about 0.74 MiB | 2 |
| 0.3.2, GPU disabled | about 1 MiB | about 1 MiB | 2 |
| 0.3.1, GPU enabled | about 24.1 MiB | about 24.1 MiB | 3 |
| 0.3.2, GPU enabled | about 24.4 MiB | about 5 MiB | 3 |

The expanded configuration, diagnostics and contracts have a measurable cost
against the older GPU-disabled provider. That version disabled GPU by default;
releasing its collector removed the much larger retained driver cost. Idle measurements recorded no
additional CPU ticks within their resolution. These Linux measurements describe
that run; they do not substitute for macOS footprint or Windows working-set data.

## NDS and clients

The existing v1 endpoint identities are retained. NDS's separately owned adapter
reads the same local stream; it does not duplicate hardware collection. The public
NDS adapter's real-provider acceptance test has read this implementation successfully.
Consumers can integrate the [JSON Schema](contracts/snapshot-v1.schema.json), the
Rust contract/client or the dependency-free Python SDK in [python/](python/).

The Python package/import identity remains `rldyour-sysinfo` / `rldyour_sysinfo`.
Published historical versions and their licenses remain immutable.

## Build and verify

The repository pins Rust, dependencies, development tools and design resources.
On a build host, `just setup` prepares checksum-verified tools; `just check` reads
source without compiling; `just test` exercises real providers; `just verify`
adds the release build. `just generate` updates derived resources from their homes.
The macOS frontend additionally requires Swift 6 and AppKit.

`systeminfo-tools package` creates the native distribution; `deb` creates the
Debian packages. Native lifecycle acceptance runs on disposable CI accounts.
`ui-linux` and `ui-windows` exercise real OS presentation and capture its output.
See [quality](docs/quality.md), [native presentation](docs/native-ui.md) and
[delivery](docs/delivery.md) for scope and evidence requirements.

Current source is Apache-2.0 with [NOTICE](NOTICE) and
[third-party notices](THIRD_PARTY_NOTICES.md).
