# Native presentation

Decision 0002 approves GNOME Shell, AppKit and Windows tray presentation. The
runtime is independent of NDS. Native UI and installer qualification is tracked
in [#39](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/issues/39).

## Shared source

`config/design-system.toml` identifies the released public OpenNetwork package. Cargo
pins its exact revision and version. `systeminfo-tools generate` projects only
the used desktop colors, metrics and original brand geometry into native
resources. The source check compares generated resources with their inputs;
generated files are not edited by hand. No private design package or bundled
font is required. Native system menus and controls retain platform appearance;
custom content uses opaque OpenNetwork surfaces. OS high contrast overrides
the saved theme, and number strings retain their direction in an RTL interface.

`locales/` owns the English, Russian, Kazakh, Arabic and Chinese messages.
Generation rejects missing keys, empty translations and changed placeholders.
The snapshot schema generates the Rust contract and the thin GJS validator.

## Configuration and GNOME migration

Typed TOML and the Rust CLI own active preferences. The GNOME frontend uses
`config show --json` and `config set`; its filesystem monitor reloads changes
without polling. Controls wait for validated configuration before appearing.
Metric and appearance edits synchronize existing controls, preserving keyboard
focus. A language or readiness transition rebuilds the translated page.

The existing extension UUID remains the installation identity. On first use,
the frontend reads explicit values from its old GSettings schema and imports
them through `config migrate --legacy-json`. False values and interval zero
are preserved. A complete temporary file is committed without replacing an
existing configuration, including a concurrent writer. The old dconf values
remain available; active settings subsequently come from TOML.

## Lifecycle and acceptance

The UI waits for provider frames, bounds its buffer and enforces a frame
deadline. Invalid data or a lost connection immediately clears readings.
Only changed text and accessibility labels are assigned. A reconnect timer
exists only while disconnected. Disabling the extension cancels I/O, timers,
subprocesses, monitors and settings subscriptions.

The popup has native scrolling constrained to the current work area, including
keyboard focus visibility. A source check cannot qualify this behavior.
On a Linux build host with GNOME and a systemd user manager, run
`systeminfo-tools ui-linux` for real Shell and GTK acceptance. It creates a
private D-Bus/Wayland session and configuration, exercises migration, locales,
themes, high contrast, native clipboard, disable/re-enable and disconnection,
and captures the UI. A temporary process scope owns activated helpers and is
stopped before cleanup. Runtime evidence in `.tools/ui-linux` stays local.

AppKit uses a status item and native menus on the main actor. Its serial socket
queue has a bounded frame and cancellable dispatch source. Real ARM and Intel
acceptance opens menus, checks five locales and four themes, round-trips the
clipboard and kills the actual provider to verify all twelve cleared readings.

Windows uses a creator-thread-owned notification window, OS menus and a bounded
latest-value proxy. No WebView or polling repaint timer is resident. File-change
notifications watch atomic configuration replacement. An open popup updates
readings in place and disables copying as soon as its provider disconnects.
Native Up/Enter navigation must produce the actual command callback. The tray
icon uses a padded opaque OpenNetwork surface so every saved theme remains
legible against the OS taskbar. High contrast uses the OS symbolic treatment.
Menu density follows Windows accessibility settings; the shared saved density
field remains available to GNOME and AppKit.

Resource measurements must use release binaries and the recorded source
baseline. Budgets and architecture choices are not measurements.
