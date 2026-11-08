# Third-party notices

The macOS GPU and HID sensor readers use the same documented IOKit registry
properties and IOHID event-system access pattern as the open-source Stats
project:

- Stats, copyright © Serhiy Mytrovtsiy, MIT License,
  <https://github.com/exelban/stats>
- MenuMeters, whose Apple Silicon hardware-reader approach is credited by
  Stats, GPL-2.0 License, <https://github.com/yujitach/MenuMeters>

No Stats binary, helper, bundle, or source file is distributed with
NDDev Systeminfo. Runtime Rust dependencies and their exact versions are
recorded in `Cargo.lock`; their license metadata remains available in
the corresponding crates.

The original OpenNetwork design tokens and brand geometry are Apache-2.0,
copyright 2026 Danil Silantyev, NDDev. The exact source is pinned in
`design-system.toml`; native resource projections are generated from that
package. Native interfaces use platform fonts; no third-party font file or
third-party icon is bundled by this projection.
