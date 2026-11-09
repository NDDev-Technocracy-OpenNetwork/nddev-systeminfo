# NDDev Systeminfo

An independent, read-only Linux/macOS/Windows system-information product. Read
README.md, docs/protocol.md and docs/adr before changing its contracts.
Keep collection independent of NDS initialization, identity and credentials.

The owner approved native GNOME, AppKit and Windows tray presentation and the
bounded FFI exception in decision 0002. Only the native crate may contain unsafe
Rust. Document ownership and safety, and exercise native lifecycles on real OSes.
All other crates forbid unsafe. Never request elevated hardware access.

Use released NDDev OpenNetwork design tokens, locale catalogs and platform
accessibility preferences. Avoid timers and redraws without demand. Measure
resource changes against a recorded source baseline; never describe a budget as
a measured result. Keep unavailable metrics null and clear disconnected readings.

Run build-free `just check` on the coding host. Builds, tests, applications
and visual acceptance run on approved build hosts or public CI runners. Preserve immutable
releases, settings, disabled state and the single owner of every native endpoint.
Keep public source generic; device inventories, credentials and private runtime
evidence stay outside this repository. Track native analysis implementation and
acceptance in #55; #39 retains the completed standalone rollout record.
