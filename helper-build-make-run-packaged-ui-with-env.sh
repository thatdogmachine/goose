#!/bin/bash
# should probably be a `just` target, but for now...
set -e

just install-deps
echo 'Installed dependencies'

just make-ui
echo 'Made UI'

#just run-ui
./helper-run-packaged-ui-with-env.sh