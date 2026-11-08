# Standalone product and NDS integration

Status: accepted product direction, 8 November 2026.

NDDev Systeminfo owns metric collection, the local protocol, its CLI, native
presentation, settings, installation and releases. Linux, macOS and Windows
must work without an NDS runtime or account. NDS connects the same daemon through
its separately owned adapter; it does not duplicate collection.

Use a small Rust workspace: a protocol contract, native OS access, and the safe
application/client composition. Native access owns handles and closes them on
drop. Collection and rate calculation run once for all consumers. A disconnected
interface immediately clears stale readings.

Keep the published v1 protocol until all callers migrate. Missing hardware is
null, never an estimate. Bound frames, admission, queues, timeouts and redraws.
Do no periodic work without clients, initialize expensive providers only when
needed, and release their resources when demand ends. Measure CPU, resident
private memory, threads and wakeups against the unchanged release.

Configuration is typed TOML. Product text has five locale catalogs. NDDev
OpenNetwork tokens and the platform's accessibility settings govern presentation.
No hardware samples are sent outside the local device by this product.

Repository transfer preserves identity, commits, tags, release IDs and assets.
Installation migration preserves settings, disabled state and single ownership
of the endpoint. Consumers move before old names are retired. Historical
artifacts and their licenses remain immutable.
