# Source and license provenance

Status: accepted within the standalone product migration, 8 November 2026.

The original project history identifies Danil Silantyev as the author of the
native implementation. Original source at this revision uses Apache-2.0 with
NOTICE. Published older tags and artifacts keep the license they carried.

macOS hardware interface precedents are recorded in THIRD_PARTY_NOTICES.md.
No Stats or MenuMeters source file, binary, helper or bundle is distributed.
The new read-only SMC binding implements the native message interface; it
exposes no fan control or key writes. It replaces the old general-purpose smc
and four-char-code dependencies, including their unreviewed license metadata
and unnecessary write/conversion functionality.

Locked dependencies are checked for every supported target by cargo-deny.
Third-party code, fonts, icons and schemas keep their applicable license text
and attribution. The product consumes the public Apache-2.0 OpenNetwork design
system, not the private corporate design-system source.
