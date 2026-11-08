Historical qualification record; current releases and issue #65 are authoritative.

# Awake NVIDIA telemetry

Physical Linux acceptance found that successful NVML loading did not imply
working telemetry. With `NO_GPUS | NO_ATTACH`, initialization succeeded but
both supported PCI address forms returned `NVML_ERROR_NOT_FOUND` for an active
adapter. The same user's independent driver tool returned its actual name,
utilization and memory. With `NO_GPUS` alone, native handle lookup succeeded.

[NVIDIA's initialization contract](https://docs.nvidia.com/deploy/nvml-api/latest/api/group__nvmlInitializationAndCleanup.html)
states that the v2 initialization is lazy and device handles initialize the
selected adapter. `NO_ATTACH` explicitly suppresses attachment. The 0.3.8 fix
uses lazy initialization and selects only PCI adapters whose current runtime
state is explicitly `active`. Suspended, suspending and unknown NVIDIA states
never initiate driver access. An unknown state remains unavailable; no reading
is fabricated. Existing disabled collection still performs no discovery.

The hardware acceptance test invokes the independent driver tool in a separate
process and only for already active, explicitly selected PCI devices. Where that
reference works, the Rust collector must return its actual name and at least
one real measurement. Separate processes prevent NVML's global initialization
reference count from hiding the regression. Driverless CI still exercises
collection bounds and disabled/unknown power behavior.

The build host passed all 52 tests and Clippy. Reintroducing the original flag
in its disposable source copy made this hardware test fail on the generic
fallback name; restoring the fix passed the same isolated test. No installed
provider was replaced for that regression experiment.

The earlier resource comparisons describe enabled driver loading; they did not
assert availability of every NVIDIA reading. They remain source-specific
historical observations, rather than evidence that this initialization flag
worked. Final release and physical acceptance are recorded in issue #65.
