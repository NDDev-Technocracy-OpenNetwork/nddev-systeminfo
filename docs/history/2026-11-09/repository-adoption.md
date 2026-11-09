Historical record; current source, guides and GitHub Releases are authoritative.

# Standalone repository completion

Version 0.3.12 introduced a repository projection and explicit local
responsibility boundaries. The archived GDS projections have no
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

The following patch removes the unused repository projection and archive ledger
after retirement of their only verifier. Neither runtime nor CI reads them.
The earlier journal content stays in [dated history](release-journal.md); Git
retains the retired metadata. Accepted decisions live in `docs/adr/` and keep
their recorded meaning. Current module guides and authored configuration remain.

The patch selects the released shared CI workflows at
[0.6.26](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-ci/releases/tag/v0.6.26),
commit 75cdd0701413ce71ee3817dbed30094fb174906a. Its check and daily security
entrypoints retain the project's native platform jobs and immutable delivery.
Package version 0.3.13 is generated from the workspace; metric, settings,
history, presentation and integration behavior are unchanged. The earlier
0.3.12 release stays immutable; final device selection follows the next
qualified patch, with one physical installation.
