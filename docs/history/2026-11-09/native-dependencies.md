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

Collection, history retention, analysis algorithms and native presentation code
remain unchanged. Earlier resource and hour-long retention records retain their
exact source identities; they are not relabeled as measurements of 0.3.9.
The complete native install/configuration/UI/package matrix and immutable
delivery must pass again before selecting this patch for native installations.

Native Apple CI now records the actual Swift compiler identity before both
package and presentation qualification. A runner image label is not used as
a substitute for the compiler that produced the native application.
