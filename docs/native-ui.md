# Native presentation

Decision 0002 approves GNOME Shell, AppKit and Windows tray presentation. The
runtime is independent of NDS. Native UI and installer qualification is tracked
in [#39](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo/issues/39).

## Shared source

`design-system.toml` identifies the released public OpenNetwork package. Cargo
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
An unchanged configuration does not rebuild the interface.

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

Resource measurements must use release binaries and the recorded source
baseline. Budgets and architecture choices are not measurements.
