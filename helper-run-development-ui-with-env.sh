#!/bin/bash
# should probably be a `just` target, but for now...
set -e

export OLLAMA_TIMEOUT=1800
export GOOSE_TOOLSHIM=true
export GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest
just run-ui