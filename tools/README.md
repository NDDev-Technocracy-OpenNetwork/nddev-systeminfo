# Qualification and distribution

Owns native source checks, generated resources, packaging, installation acceptance and qualified artifact publication. These are build-time tools, not resident product processes.

See the [product guide](../README.md), [protocol](../docs/protocol.md) and
[quality contract](../docs/quality.md). Root `just check` reads source;
native behavior and packaging are qualified on build hosts and CI.

The source-owned prepare-release BASE_SHA command prepares only the next patch
and existing derived metadata before qualification. Its base must be the normal
update's original full commit. Publication remains exclusively with publish
after exact CI/artifact qualification; preparation never creates refs or assets.
