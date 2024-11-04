#!/bin/bash

set -e

cargo fmt --check
cargo clippy --workspace -- --deny warnings
cargo nextest run
