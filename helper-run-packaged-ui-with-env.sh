#!/bin/bash
# should probably be a `just` target, but for now...
set -e

OLLAMA_TIMEOUT=1800 GOOSE_TOOLSHIM=true GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest open -a /Users/$(whoami)/repos/goose-tdm-fork/ui/desktop/out/Goose-darwin-arm64/Goose.app

