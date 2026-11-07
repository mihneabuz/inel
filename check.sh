#!/usr/bin/env bash
# Formats, lints and runs the tests on every backend: the real kernel, the
# fake natively, then the fake under miri with several seeds and both
# borrow models.
#
# Usage: ./check.sh [test filter]
#   SEEDS="1 2 3"  seeds for the miri runs (miri's and the fake's)
set -euo pipefail
cd "$(dirname "$0")"

SEEDS=${SEEDS:-"1 2 3 4"}
FILTER=("$@")

step() {
    printf '\n\033[1;34m==> %s\033[0m\n' "$*"
}

step "fmt"
cargo fmt --all

step "clippy"
cargo clippy --workspace --all-targets

step "test (kernel)"
cargo test --workspace -- "${FILTER[@]}"

# separate target dir so the different rustflags don't trigger full rebuilds
step "test (fake)"
CARGO_TARGET_DIR=target/fake \
    cargo test --workspace --config 'build.rustflags=["--cfg","inel_fake"]' \
    -- "${FILTER[@]}"

# miri hides the host environment, the fake's seed has to be forwarded
miri_flags="-Zmiri-env-forward=INEL_FAKE_SEED"

for seed in $SEEDS; do
    for borrows in stacked tree; do
        flags="$miri_flags -Zmiri-seed=$seed"
        [[ $borrows == tree ]] && flags="$flags -Zmiri-tree-borrows"

        step "miri (seed $seed, $borrows borrows)"
        INEL_FAKE_SEED=$seed MIRIFLAGS="$flags ${MIRIFLAGS:-}" \
            cargo +nightly miri test --workspace -- "${FILTER[@]}"
    done
done

step "all checks passed"
