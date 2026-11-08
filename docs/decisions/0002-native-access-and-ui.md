# Native OS access and lightweight presentation

Status: accepted by the owner, 8 November 2026. Native presentation and the
bounded FFI exception were explicitly approved.

## Native access

Propose one private-by-default Rust native crate for the existing Mach, BSD,
IOKit, AppleSMC/IOHID and socket-activation bindings, plus Windows tray bindings.
Only that crate may contain unsafe Rust. The protocol, configuration, sampler
scheduler, public client and CLI forbid unsafe code.

Every binding documents pointer validity, ownership, thread requirements and
cleanup. Real-platform tests exercise success, missing hardware, invalid handles
where the OS permits them, repeated creation/drop and shutdown. No unsafe access
is introduced into NDS's adapter. Device access remains read-only and needs no
administrator privileges.

Windows presentation lifecycle includes starting and stopping the owned tray and
provider. Background launch uses a Win32 handle allowlist containing only null
stdin and the local JSON log. It never inherits a caller's pipes or unrelated
handles. Rust's corresponding attribute-list interface is still unstable in
the pinned stable toolchain. The native crate owns the temporary attribute list,
inheritable duplicates and process/thread handles; the rest of the product uses
one safe function. Native installation acceptance captures CLI output and must
finish while the background products remain alive.

The installed Windows CLI unlinks its own owned image with documented POSIX
file-disposition semantics before removing its version directory. The mapped
image remains valid through process exit. This requires a local filesystem
supporting that Windows operation; failures remain failures. It introduces no
cleanup script, helper copy, reboot or temporary process. Real self-uninstall
acceptance, rather than a source check, qualifies this boundary.

References: [Rust CommandExt](https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html),
[Windows handle inheritance](https://learn.microsoft.com/en-us/windows/win32/procthread/inheritance).
See [Windows POSIX file disposition](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/2e860264-018a-47b3-8555-565a13b35a45).

## Presentation options

The default desktop stack is Tauri with React/TypeScript. It supplies one shared
interface across Linux, macOS and Windows, with a WebView process for the window.

The proposed exception retains GNOME Shell's existing thin GJS indicator and
AppKit's existing thin Swift menu-bar client, and adds a native Windows tray
client. Each consumes the same socket stream, shared locale catalogs and released
OpenNetwork tokens. System menus keep system colors; custom content uses opaque
NDDev surfaces. Keyboard navigation, accessible labels, RTL, high contrast and
reduced motion remain required.

The reason is the actual workload: a small status surface updating a few numbers
on daemon events. Native presentation avoids keeping a WebView or a continuously
rendered GPU surface resident. This benefit is a hypothesis until measured on the
supported platforms; record measurements and reject regressions before release.

This choice affects the UI implementation only. Standalone behavior, protocol,
three-OS collection, migration and resource qualification are required in either
case.
