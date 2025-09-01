#!/bin/bash
# should probably be a `just` target, but for now...
set -e

echo 'Cleaning cargo build artifacts'
cargo clean
echo 'Cargo build artifacts cleaned'

echo 'Cleaning ui/desktop/node_modules/'
rm -rf ./ui/desktop/node_modules/
echo 'ui/desktop/node_modules/ cleaned'
