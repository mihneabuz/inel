#!/bin/bash

set -e

cargo fmt --check
cargo clippy --workspace --no-default-features
cargo clippy --workspace --all-features -- --deny warnings
cargo nextest run --all-features
