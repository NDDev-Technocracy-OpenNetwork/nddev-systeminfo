# Checks that read source only; safe on the coding host.
check:
    cargo fmt --check --manifest-path daemon/Cargo.toml
    ./scripts/check-extension.sh
    ./scripts/check-version.sh
    git diff --check
