History of the implementation decisions on this date; not a statement of current release status.

# Standalone Systeminfo

The requested result is an independent Linux/macOS/Windows tool in OpenNetwork,
ready for NDS integration, following NDDev standards with low resource use. The
owner selected native desktop presentation and approved a bounded native Rust
FFI crate for OS access and presentation lifecycle.

The original product repository moved to
`NDDev-Technocracy-OpenNetwork/nddev-systeminfo` with its identity, history,
historical tags and release assets retained. OpenNetwork's public Apache design
system moved to its canonical organization; NDS uses a distinct fork. Systeminfo
pins the public original. Its runtime remains independent of NDS initialization,
identity and credentials; the separately owned thin NDS adapter reads the stable
local v1 contract.

Implementation is tracked by #39: #44 establishes the standalone contracts and
configuration; #45 bounds native collection and transport; #47 adds shared/GNOME
presentation; #48 qualifies AppKit; #49 qualifies the Win32 tray; #50 owns native
installation and package lifecycle; #51 owns qualified artifact reuse and delivery.
The parent issue records exact revisions, qualification and remaining work.

Actual native surfaces, five languages, four themes, cancellation, unavailable
readings and disconnected menus are part of acceptance. The Linux resource
comparison in `docs/quality.md` records both the reduced GPU/default footprint
and the extra non-GPU cost; budgets are not substituted for measurements.

Windows denied removal of a mapped running image despite POSIX disposition
flags. The implementation therefore delegates temporary cleanup-executable
lifetime to the OS's per-user Windows Installer, using a small Rust-generated
maintenance package. No resident helper, elevation, shell cleanup or reboot is
part of the product. The installed CLI exits before cleanup removes its image.

Delivery promotes the exact qualified dev revision and republishes its existing
native/Python/Debian artifacts. Downloaded release bytes are compared before
publication and signed APT distribution. macOS uses an ad-hoc signature until a
product Developer ID/notarization identity is configured. Current status and
release evidence must be read from #39 and the repository's release records.

The first 0.3.2 publication attempt (CI 42489391606) stopped before promotion or
tag creation: the CI archive endpoint requires a JSON media type, whereas a
release-asset download requires octet-stream. The version remains reserved and
the corrected distribution proceeds as 0.3.3. The collector behavior is unchanged.
The corrected requests were exercised against an actual CI archive and the
historical 0.3.1 Linux release asset; both matched GitHub's recorded SHA-256.
Ubuntu workspace tests and Clippy, plus build-free source checks, passed for
the correction before cross-platform qualification.

The first physical Debian rollout exposed a multi-user restart defect in the
distribution's invoke helper: its admitted unit list persists across user
managers, so an enabled account can affect a later masked account. Product #53
scopes restart admission per manager while retaining Debian enablement and
service policy. The new Debian acceptance uses two actual user managers, with
one enabled and one masked; 0.3.4 carries this package-only correction.
