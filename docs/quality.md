# Runtime and platform qualification

The runtime separates the contract, safe application code and bounded native
access. Only `crates/native` may contain unsafe Rust. Its APIs own and release
handles, Mach rights, IOKit objects, windows, menus and background-process
inheritance. Hardware collection never initializes NDS or requests elevation.

One lazy collector serves at most 32 clients and 32 queued admissions. A 256-byte
handshake and an absolute 250 ms deadline bound admission. Frames are limited
to 4096 bytes, sampling to a 500 ms floor, and slow readers disconnect rather
than block publication. Every v1 metric is present and nullable; nonfinite or
out-of-range readings are rejected at the boundary.

After thirty idle seconds the collector releases its native resources and GPU
library. Socket-activated Linux/macOS processes exit; standalone and Windows
listeners block without polling. Cancellation shuts down the actual client
socket. A second instance never unlinks another live listener, a regular file
or a symlink. UI disconnection clears all twelve readings immediately.

`just check` performs build-free source, generated-contract, formatting, license
and workflow checks. Build hosts and public CI run `just test` and `just verify`.
The pinned toolchain lives in `rust-toolchain.toml`; no local application or
build is required on the coding host.

CI qualifies Linux x64, macOS Apple silicon/Intel and Windows x64, with Clippy
and tests including builds without NVIDIA. Real-provider regressions cover
cadence, silent/oversized peers, connection churn, idle release and bounded
admission. Parallel configuration writers must preserve each other's fields.
The dependency-free Python SDK is tested on all three OSes; its actual socket
client runs where CPython provides AF_UNIX, currently Linux and macOS. The
native Rust client exercises Windows sockets.

Native UI acceptance opens actual GNOME Shell/GTK, AppKit and Win32 surfaces.
It checks the five locales, four themes, RTL, unavailable readings, persisted
preferences, lifecycle and disconnection. Captures are downloaded and visually
reviewed. GNOME 46–50, macOS 12+ and x64 Windows 10/11 are the supported surfaces;
additional hardware readings remain model-dependent and unavailable ones are
`null`. Passing on a runner does not certify every firmware or GPU driver.

Installation acceptance runs only on disposable CI accounts. It installs the
native package, changes zero/false settings, disables the provider, reinstalls,
checks preservation, enables and samples the real provider, applies settings
and invokes uninstall from the installed CLI. Uninstall preserves user TOML.
Debian tests run native maintainer helpers in an isolated `DPKG_ROOT`, then
install both real packages and exercise disabled upgrades and purge on Ubuntu.
Two actual user managers qualify independent restart admission: an enabled
account precedes a masked account, and the package must preserve both states.
Successful qualification explicitly removes its temporary users and managers.

The [delivery contract](delivery.md) reuses qualified artifacts through stable
promotion and checks their downloaded release bytes before signed APT
publication. It never treats a passing PR build as release qualification.

## Resource evidence

The Linux comparison uses release builds, separate executable copies, realtime
demand, ten seconds active and ten seconds idle after collector release. PSS
comes from `/proc/<pid>/smaps_rollup`; CPU is the change in process CPU ticks.
The old source is the immutable v0.3.1 release. GPU-disabled active PSS changes
from about 754 KiB to about 1045 KiB. The new default disables GPU, whereas the
old default enabled it at about 24658 KiB. With GPU enabled, the new provider
releases from about 24952 KiB to 5104 KiB after idle, versus about 24659 KiB
retained by the old provider. Two threads serve the GPU-disabled case; three
serve the active GPU case.

Observed first frames were 1.28 ms old / 1.95 ms new without GPU and 18.17 ms old
/ 16.55 ms new with GPU. CPU counters advanced zero or one tick during the
active ten-second samples and zero in the idle observation. These are one-host
observations, with a visible non-GPU footprint cost, not universal latency,
zero-CPU or leak-free guarantees. RSS, PSS and macOS physical footprint are
different measurements and must not be compared as the same quantity.
