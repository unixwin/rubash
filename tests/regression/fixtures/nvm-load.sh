#!/usr/bin/env bash
# nvm.sh loader semantics (ecosys5 lane corpus, nvm-sh/nvm master 5226
# lines, MIT). GNU-diff golden: source the real loader into a fixture
# NVM_DIR and check the load contract both byte-pin: rc 0, `nvm' is a
# function, `nvm --version' prints the real version, and the nvm_*
# function family count. The file is vendored at
# tests/regression/corpus/nvm/nvm.sh (same bytes as the lane corpus).
# Capture (run from a scratch copy of this directory WITH nvm.sh):
#   MSYS_NO_PATHCONV=1 wsl bash -c \
#     'cd <scratch> && export THIS_SH=bash && /usr/local/bin/bash nvm-load.sh' \
#     > nvm-load.gnu.out 2> nvm-load.gnu.err
# (write captures OUTSIDE the scratch dir; see tests/regression.rs)
export NVM_DIR="$PWD/nvmdir"
mkdir -p "$NVM_DIR"
. ./nvm.sh
echo "rc=$?"
type -t nvm
nvm --version 2>/dev/null  # harness PATH has no tr; nvm() computes DEFAULT_IFS via `command tr` at entry
echo "FUNCS=$(declare -F | grep -c ' nvm_' || true)"
echo done
