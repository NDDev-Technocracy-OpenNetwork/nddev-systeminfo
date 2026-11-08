#!/usr/bin/env bash
# rldyour-sysinfo — build the APT package
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Usage: scripts/build-deb.sh <rldyour-sysinfod binary> <output directory>
#
# The package places the daemon in /usr/bin, so the service unit is rendered
# from the repository copy with its ExecStart pointed at that path — the
# source tree's unit keeps pointing at the ~/.local/bin layout install.sh
# produces.
set -euo pipefail

binary="$(realpath "$1")"
outdir="$2"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${root}/Cargo.toml" | head -1)"

scratch="$(mktemp -d)"
trap 'rm -rf "${scratch}"' EXIT
stage="${scratch}/stage"
install -d -m755 "${stage}" "${scratch}/debian"
# Generate native shared-library dependencies rather than letting a binary
# requiring newer glibc install on an incompatible distribution.
printf 'Source: rldyour-sysinfo\nMaintainer: NDDev OpenNetwork <danil@nddev.it.com>\n\n' > "${scratch}/debian/control"
cat "${root}/packaging/deb/control" >> "${scratch}/debian/control"
shlibs="$(cd "${scratch}" && dpkg-shlibdeps -O -e "${binary}")"
shlibs="${shlibs#shlibs:Depends=}"
[[ -n "${shlibs}" ]] || { echo 'Missing shared-library dependencies' >&2; exit 1; }

install -Dm755 "${binary}" "${stage}/usr/bin/rldyour-sysinfod"
install -Dm644 "${root}/daemon/systemd/rldyour-sysinfod.socket" \
    "${stage}/usr/lib/systemd/user/rldyour-sysinfod.socket"
sed 's|^ExecStart=.*|ExecStart=/usr/bin/rldyour-sysinfod|' \
    "${root}/daemon/systemd/rldyour-sysinfod.service" \
    > "${stage}/usr/lib/systemd/user/rldyour-sysinfod.service"
chmod 644 "${stage}/usr/lib/systemd/user/rldyour-sysinfod.service"

install -Dm644 "${root}/packaging/deb/control" "${stage}/DEBIAN/control"
sed -i "/^Package: rldyour-sysinfo$/a Version: ${version}" "${stage}/DEBIAN/control"
sed -i "s|^Depends:.*|Depends: init-system-helpers, ${shlibs}|" "${stage}/DEBIAN/control"
install -Dm755 "${root}/packaging/deb/postinst" "${stage}/DEBIAN/postinst"
install -Dm755 "${root}/packaging/deb/prerm" "${stage}/DEBIAN/prerm"
install -Dm755 "${root}/packaging/deb/postrm" "${stage}/DEBIAN/postrm"

install -d "${outdir}"
dpkg-deb --root-owner-group --build "${stage}" \
    "${outdir}/rldyour-sysinfo_${version}_amd64.deb"
