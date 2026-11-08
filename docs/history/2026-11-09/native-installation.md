Historical qualification record; current releases and issue #63 are authoritative.

# macOS active upgrade transition

The immutable 0.3.6 release passed native CI, delivery and signed APT. A physical
active Mac upgrade then returned launchctl bootstrap exit 5 after bootout.
Payload signature verification and generated plist syntax passed. Native launchd
recorded removal of both product labels 31 ms after the failed registration
attempt. Explicit later native service start and UI bootstrap restored the jobs.
The actual provider returned valid logical-core and Apple GPU readings.

This run demonstrates a transition race, not a universal diagnosis for exit 5.
Issue #63 adds backoff starting at 25 ms, capped at 500 ms, with a total ten-second
bootstrap deadline. Other errors fail immediately and the final error names the
operation/label. The implementation preserves launchd ownership and disabled
state, with no elevated or detached-provider fallback.

Native Mac installation qualification now performs four active upgrade cycles,
checking a loaded provider, a running native UI and actual local samples in
addition to the existing disabled-state, preferences and removal journeys.
The next patch is 0.3.7; previously published tags and assets remain immutable.
