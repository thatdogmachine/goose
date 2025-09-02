#!/bin/bash
# should probably be a `just` target, but for now...
set -e

export OLLAMA_TIMEOUT=1800
export GOOSE_TOOLSHIM=true
export GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest
open -a /Users/$(whoami)/repos/goose-tdm-fork/ui/desktop/out/Goose-darwin-arm64/Goose.app

