Historical work record; current contracts, issues and releases are authoritative.

# Native metric analysis

The requested result is correct GPU availability, clickable OpenNetwork metric
cells, logical-core CPU detail, a one-hour analysis window and an explicit
brand website action. Work is tracked by #55 and implementation slices #56,
#57 and #58.

Stats' current GPU reader was reviewed as an MIT-licensed behavior/API reference.
It uses IOAccelerator PerformanceStatistics, the same passive utilization keys
our cached Rust reader already consumes. Native Apple-silicon inspection found
the counters available while collection was disabled. The UI must distinguish
that state from unsupported hardware and from a real zero. Unified memory is
reported as shared bytes, never invented dedicated-VRAM percentage.

CPU details reuse the existing native counter read. Optional detailed frames
and bounded history do not enlarge the ordinary v1 stream used by integrations.
History is a rolling RAM buffer, with age/count/byte limits and explicit gaps;
analysis does not create a database or recurring disk files. Plot preparation
and statistics remain in Rust. The three native interfaces consume one model.

Reviewed primary material:
- https://github.com/exelban/stats/blob/master/Modules/GPU/reader.swift
- https://developer.apple.com/documentation/kernel/1502854-host_processor_info
- https://developer.apple.com/documentation/iokit/1514537-ioregistryentrysearchcfproperty
- https://docs.kernel.org/gpu/amdgpu/thermal.html
- https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/
- https://learn.microsoft.com/en-us/windows/win32/api/pdh/nf-pdh-pdhgetformattedcounterarrayw
- https://docs.rs/nvml-wrapper/0.13.0/nvml_wrapper/struct.Nvml.html

These sources establish API behavior, not universal firmware support or resource
measurements. Native qualification and release measurements belong to #55.
