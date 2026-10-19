#!/usr/bin/env bash
# Perf probe 31: the #800A ticket face — kiddae/dotfiles hermine .bashrc pays
# ~32 `$(tput …)` command substitutions while sourcing (source_wall 9.7s on
# the eco-harvest runner ≈ ~300ms/spawn). This loop mirrors the asset's own
# shape: 16 color groups × (`tput bold` + `tput setaf N`) = 32 tput calls,
# TERM=xterm as the harvest harness sets.
# Shape family: command-substitution external spawn loop (#800A tput face).
# PERF: runs=7 timeout=60
export TERM=xterm
i=0
while [ "$i" -lt 16 ]; do
  Red="$(tput bold)$(tput setaf 1)"
  Green="$(tput bold)$(tput setaf 2)"
  i=$((i+1))
done
exit 0
