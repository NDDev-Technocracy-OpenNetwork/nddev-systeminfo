Historical record; current manifests, source and GitHub Releases are authoritative.

# Standalone repository completion

The standalone product adopts its current `.nds/nddev.yaml` manifest and
explicit local responsibility boundaries. The archived GDS projections have no
NDS runtime reader and are retired rather than renamed into another format.
GitHub repository identity, immutable versions and live v1 consumers remain.
An initial provenance repair was superseded after checking the actual reader
retirement and current manifest contract; no GDS runtime or shared policy was
changed.

Canonical product, design selection and delivery inputs live under `config/`.
They keep their native readers and generated projections. The accepted native
presentation, FFI and provenance decisions gain machine-readable status/date
without changing the recorded decisions. Local module guides explain ownership
and focused verification.

GitHub Releases is the current release journal. The native archive operation
preserves the earlier local journal's original bytes and provenance. Publication
requests notes for the exact qualified commit before any tag or stable ref is
changed; recovery retains the existing hosted release body. Distribution
licenses and notices remain shipped.

GNOME metric cells launch analysis with GIO startup notification so the
compositor can activate the requested window. The native acceptance fixture
uses a path with a space and percent sign and requires focus without manually
raising the analysis window. This changes no metric collection or history
policy. See the [GIO launch contract](https://docs.gtk.org/gio/vfunc.AppInfo.launch.html).

Source and structural checks precede native CI on Linux, macOS ARM/Intel and
Windows. Native installed acceptance remains separate from structural
conformance; CI does not certify every physical GPU or firmware.
