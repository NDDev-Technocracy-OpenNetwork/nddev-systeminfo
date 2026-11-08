# Changelog

## 0.3.5

- Add passive per-adapter GPU details and logical-core CPU readings on Linux,
  macOS and Windows. New configurations enable GPU collection; existing explicit
  settings remain intact. Apple unified memory is reported as shared bytes.
- Add negotiated, bounded, RAM-only one-hour history and shared Rust analysis
  with peak-preserving plots, time-weighted statistics and explicit gaps.
- Add native metric cells and analysis windows, core pages, ranges, pause/resume,
  JSON export and the OpenNetwork website action, using released themes and all
  five locales. Closing a window cancels its reader.
- Preserve ordinary v1 SDK/NDS streams and keep history opt-in for integrations.
  Add an installed native UI launcher and update qualified Python/CI tooling.

## 0.3.4 — 2026-11-08

- Admit Debian package restarts independently for each active user manager,
  preserving masked and disabled secondary accounts and service policy.
- Qualify actual enabled and masked systemd user managers in Debian CI.

## 0.3.3 — 2026-11-08

- Publish the independent NDDev Systeminfo product in OpenNetwork, preserving
  repository history, historical releases and the local v1 consumer contract.
- Add the standalone Rust CLI, complete typed TOML, atomic field updates and
  migration of explicit legacy preferences.
- Use the released public OpenNetwork design system in native GNOME, AppKit and
  Windows interfaces, with five languages, RTL, OS accessibility and immediate
  clearing of disconnected readings.
- Isolate native ownership and FFI, replace the macOS hardware bridge with
  read-only Rust access and exercise native handle and socket lifecycles.
- Disable GPU collection by default, release collectors when demand ends, reuse
  frame buffers and keep idle listeners free of sampling timers.
- Add native user installation, upgrade and removal with preserved preferences
  and disabled state, plus canonical and transitional Debian packages.
- Qualify actual interfaces, native startup managers, CLI and NDS adapter use.
  Publish qualified artifacts with provenance and byte verification, then sign APT.
- Use each GitHub binary endpoint's required media type when downloading CI
  archives and release assets.

## 0.3.2 — unpublished, 2026-11-08

Native qualification passed at `a12da58e52a989a451ae0fbb3a8622044e08da37`.
Publication stopped before promotion or tag creation because the CI archive
endpoint rejected a release-asset media type. The version remains reserved;
the corrected publisher and standalone distribution ship as 0.3.3.

## 0.3.1 — 2026-10-07

- Enable the Debian user socket on fresh installation and preserve explicit
  administrator disablement on upgrades, following the native helper lifecycle.
- Test the real helper in an isolated package root and query global systemctl
  state after installing the package in CI.
- Publish signed APT only after an existing stable release, preserve the
  main-only deployment environment and make public indexes readable to _apt.

## 0.3.0 — 2026-10-07

- Separate the bounded streaming server, platform socket activation and metric
  collectors. Cached first samples and one sampling clock prevent connection
  churn from forcing extra hardware reads. Slow readers cannot stall peers.
- Bound handshakes to 256 bytes and 250 ms total, admit at most 32 clients,
  preserve a live listener on duplicate startup and validate inherited sockets.
- Stop sampling while no clients are attached; preserve socket activation and
  the thirty-second idle exit on Linux and macOS.
- Track Linux disk/network counters per physical device, discover hotplugged
  devices every thirty seconds and use successful-observation intervals after
  read failures. Read only the aggregate prefix of /proc/stat.
- Replace the C HID temperature shim with Rust RAII, cache HID clients/services,
  reduce SMC metadata reads and reuse Mach host rights with explicit release.
  Handle wrapping 32-bit Mach CPU counters without a false idle sample.
- Preserve independent NVIDIA readings when one NVML metric is unavailable.
- Split the macOS UI, typed protocol/formatting and event-driven socket client.
  Enforce Swift 6 concurrency, prevent SIGPIPE, bound incoming frames and skip
  unchanged UI labels. No blocked reader thread is kept resident.
- Bound GNOME and Python frames; clean up cancelled GNOME connections and
  perform the handshake write asynchronously.
- Update Windows sysinfo to 0.39.6, require Rust 1.95 and Swift 6 for macOS,
  and extend cross-platform lifecycle, socket and packaging checks.
- Link the macOS daemon against the system Swift runtime search path so the
  IOKit bridge also loads in standalone Intel builds without Xcode paths.

## 0.2.1 — unreleased changes incorporated into 0.3.0

- Add cadence modes end to end: clients may ask for `interval` 1–60 seconds
  or `0`, which selects realtime — the daemon ticks every 500 ms, still above
  sysinfo's minimum CPU refresh window. The extension preferences offer
  Economy, Standard, Fast and Realtime presets; the macOS app honours
  `defaults write com.nddev-opennetwork.rldyour-sysinfo interval N`; and
  `RLDYOUR_SYSINFO_INTERVAL=0` runs realtime when no client asks.
- Trim the remaining per-tick allocations in the macOS and Windows readers:
  interface classification and sensor averaging no longer allocate per
  sample.
- Move the macOS socket to `~/Library/Application Support` (Caches is
  purgeable) and activate the daemon through a launchd `Sockets` entry, so it
  idles out like the systemd unit instead of running continuously.
- Restrict self-bound sockets to the owner (0600), remove stale socket files
  before binding, and keep `XDG_RUNTIME_DIR` from redirecting the socket on
  macOS and Windows.
- Count Linux CPU ticks once by excluding guest time already inside
  user/nice, and convert Windows network and volume counters into the
  bytes-per-second the protocol specifies; Windows now reports disk
  throughput and filters software adapters.
- Split the macOS and Windows collectors into the same per-domain module
  layout Linux uses, with one shared NVML reader honouring
  `RLDYOUR_SYSINFO_GPU=0` on both platforms.
- Check the protocol version in every client, align the menu-bar app's
  formatting and reconnect defaults with the extension, and send the Python
  handshake as an integer.
- Enable the APT package's user socket on install via maintainer scripts and
  point its service at `/usr/bin`; write proper metadata into the repository
  `Release` file.
- Register the Windows daemon through the `Run` key as a windowless process
  instead of a console-flashing Startup shim.

## 0.2.0 — 2026-09-12

- Publish the Rust daemon on crates.io and a dependency-free protocol client
  on PyPI.
- Add a signed APT repository for Ubuntu amd64, hosted on GitHub Pages.

- Add a native macOS collector for CPU, memory, Apple GPU, disk, network and SMC temperatures.
- Add a native macOS menu bar client and per-user LaunchAgent installation.
- Add a Windows collector for CPU, memory, swap, network, temperatures and optional NVIDIA metrics.
- Split collectors into explicit `linux`, `macos` and `windows` modules.
- Make the local socket transport work on Windows 10 and newer.
- Verify daemon builds on Ubuntu, macOS and Windows in CI and publish platform archives.

## 0.1.0 — 2026-08-14

- Add the lightweight Linux daemon and GNOME Shell panel extension.
