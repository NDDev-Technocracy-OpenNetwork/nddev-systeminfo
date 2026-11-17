# Prepare tools on a build host or CI. Source checks below never compile.
setup:
    cargo run --locked --package systeminfo-tools -- setup

# Checks that read source only, offline; safe on the coding host.
check:
    just --fmt --check
    cargo fmt --all -- --check
    ./.tools/bin/cargo-deny{{ exe }} deny --frozen check
    ./.tools/bin/cargo-shear{{ exe }} --frozen
    ./.tools/bin/systeminfo-tools{{ exe }} check

test:
    ./.tools/bin/systeminfo-tools{{ exe }} test

generate:
    ./.tools/bin/systeminfo-tools{{ exe }} generate

verify: check test
    cargo build --release --workspace --all-features --locked

exe := if os() == "windows" { ".exe" } else { "" }

# Prepare the next source-owned patch from the updater original base, without publishing.
prepare-release base:
    ./.tools/bin/systeminfo-tools{{ exe }} prepare-release {{ quote(base) }}
