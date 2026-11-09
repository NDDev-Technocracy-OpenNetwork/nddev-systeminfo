# Provider and native clients

Read [README.md](README.md) and the [root contract](../AGENTS.md).

Keep the engine independent of NDS credentials and initialization. Preserve v1 identities, preferences and explicit disablement. Call only safe APIs from crates/native. No polling without demand; close resources on cancellation and idle release.
