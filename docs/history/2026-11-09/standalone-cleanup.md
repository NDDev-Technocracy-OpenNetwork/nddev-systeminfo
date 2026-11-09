Historical cleanup record; current code, releases and issue #70 are authoritative.

# Retire obsolete standalone artifacts

The post-release audit found two stale per-agent memories describing the old
collector layout, release commands and platform support. Current README,
protocol, native UI, delivery and quality guides already cover the maintained
behavior. The duplicate memories were removed instead of retaining another
project description.

Mac launch templates still named retired executables, while the installer
rewrote those paths to the names actually shipped. The templates now describe
the shipped app and provider directly; only the user's escaped home path is
substituted. Native application source and the compiler-check output use the
standalone product name. The generated installed launch configuration remains
the same, including its socket, labels and disabled-state policy.

The surviving historical names have current consumers: v1 socket identities,
native manager labels, the Python package/import, explicit settings migration
and the Debian upgrade bridge. None implements a second collector. Published
releases and their licenses remain immutable. Superseded dependency requests
for the old workspace are reconciled in issue #70.

Source checks and the existing real native installation, repeated-upgrade,
sampling and UI matrix qualify this patch before publication. Runtime cleanup
uses explicit ownership and reference checks; private device inventories and
receipts stay outside this repository.

A second media pass found docs/panel.png, an unreferenced image of the earlier
panel. It was removed from current documents; Git retains the historical asset.
No application, contract or native lifecycle behavior changes with its removal.

Routine dependency proposals now use a three-day settling window in each
existing Cargo, Actions and Python updater entry. Package and action pins
continue to be qualified before adoption.
