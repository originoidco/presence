#!/usr/bin/env bash
set -e

# i cant be bothered to make this pretty
echo "! Starting Redis"
docker compose up -d --wait

echo "! Redis ready, starting presence
cargo run
