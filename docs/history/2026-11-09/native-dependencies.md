Historical qualification record; current releases and issue #68 are authoritative.

# Compatible TOML patch cohort

The working 0.3.8 release remains available while the next patch qualifies a
specific dependency family. TOML 1.2's published Cargo manifest accepts
serde_spanned and toml_datetime from 1.1.2. The lock now deliberately selects
those compatible patches instead of 1.1.3, without introducing a direct
application dependency, new FFI or a protocol change.

Cargo resolved both precise selections. Build-free source/advisory/license
checks passed, and the build host passed all 52 tests and Clippy, including
real-provider configuration updates, invalid-input preservation, native
sampling, bounded replay/analysis and the independent NVIDIA driver comparison.

All-feature workspace graphs were inspected for Linux x64, macOS ARM/Intel and
Windows MSVC x64. EFI, GNU Windows, WebAssembly companion and objc2-io-kit names
present only in the complete lock were absent from these four enabled graphs;
they are not new activated runtime technologies. A Rust package's registry
identity remains distinct from an unrelated npm package with the same name.

Collection, history retention and analysis algorithms remain unchanged.
Native presentation behavior is preserved. Earlier resource and hour-long retention records retain their
exact source identities; they are not relabeled as measurements of 0.3.9.
The complete native install/configuration/UI/package matrix and immutable
delivery must pass again before selecting this patch for native installations.

Native Apple CI now records the actual Swift compiler identity before both
package and presentation qualification. A runner image label is not used as
a substitute for the compiler that produced the native application.

The first measured image default was Apple Swift 6.3.3. The accepted compiler
can be selected independently with Swift.org's official 6.4.2 signed macOS
package, installed into the build user's Toolchains directory. Its streamed
SHA-256 is pinned in tools/swift.toml; the installer also checks the publisher
identity specified by the official package instructions. Temporary downloads
are removed on success or failure. No user-device SDK or global Xcode selection
is changed, and no Swiftly dependency is introduced.

Production packaging checks the actual compiler version, explicitly excludes
toolchain standard-library rpaths and inspects Mach-O linkage for compiler
bundle paths. Native installation and AppKit tests must run against system
libraries, rather than accidentally relying on a compiler installed on CI.
The native matrix qualifies this selected compiler with the runner's existing
SDK before publication; this record does not predeclare its result.

Primary sources: [Swift package installation and publisher identity](https://www.swift.org/install/macos/package_installer/)
and [Swift driver rpath options](https://github.com/swiftlang/swift-driver/blob/main/Sources/SwiftOptions/Options.swift).

The first native attempt installed and selected Swift 6.4.2 successfully, then
reported no AppKit module because the standalone driver did not infer the
image's SDK. The helper now passes the existing xcrun macOS SDK path explicitly
through SDKROOT, without selecting or installing a different SDK. Native
qualification covers this binding before the patch can publish.

With the SDK bound, Swift 6.4.2 imports AppKit and checks the actual sources.
Its stricter capture diagnostic requires the startup queue closure to declare
its existing strong self capture explicitly; the descriptor event handler
remains weak to prevent a cycle. This preserves startup/stop ownership and is
qualified by the native pause/close and repeated-upgrade journeys.
