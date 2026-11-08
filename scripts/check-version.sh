#!/usr/bin/env bash
# rldyour-sysinfo — version consistency gate
# SPDX-License-Identifier: AGPL-3.0-or-later
#
#   scripts/check-version.sh          daemon == pyproject == __version__ == changelog
#   scripts/check-version.sh v1.2.3   plus: the tag matches all of the above
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

daemon="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${root}/Cargo.toml" | head -1)"
pypi="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${root}/python/pyproject.toml" | head -1)"
client="$(sed -n 's/^__version__ = "\([^"]*\)"/\1/p' "${root}/python/src/rldyour_sysinfo/__init__.py" | head -1)"

if [[ -z "${daemon}" || "${daemon}" != "${pypi}" || "${daemon}" != "${client}" ]]; then
    printf 'version drift: daemon=%s pyproject=%s client=%s\n' \
        "${daemon:-unset}" "${pypi:-unset}" "${client:-unset}" >&2
    exit 1
fi

# During development the version may sit under "Unreleased"; once released it
# must appear as its own heading.
if ! grep -qE "^## (Unreleased|${daemon})( |$)" "${root}/CHANGELOG.md"; then
    printf 'CHANGELOG.md has no entry for %s\n' "${daemon}" >&2
    exit 1
fi

if [[ $# -eq 1 ]]; then
    if [[ "$1" != "v${daemon}" ]]; then
        printf 'tag %s does not match version %s\n' "$1" "${daemon}" >&2
        exit 1
    fi
fi

printf 'version %s is consistent\n' "${daemon}"
