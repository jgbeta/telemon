#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo test
cargo run -- check --config config.example.yml
cargo run -- discover --config config.example.yml
cargo run -- print-metrics --config config.example.yml
