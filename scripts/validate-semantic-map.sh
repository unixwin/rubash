#!/usr/bin/env sh
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
set -eu

map="docs/semantic-ownership.tsv"
test -f "$map"

awk 'BEGIN { FS = sprintf("%c", 9) }

  NR == 1 {
    if ($1 != "GNU source family" || NF != 7) {
      print "invalid semantic map header" > "/dev/stderr"
      exit 1
    }
    next
  }
  NF != 7 {
    print "line " NR ": expected 7 tab-separated fields" > "/dev/stderr"
    bad = 1
  }
  $4 !~ /^(active|unreferenced|missing)$/ {
    print "line " NR ": invalid compile status: " $4 > "/dev/stderr"
    bad = 1
  }
  $5 !~ /^(real|partial|scaffold|bridge|deferred|host-owned)$/ {
    print "line " NR ": invalid implementation status: " $5 > "/dev/stderr"
    bad = 1
  }
  $6 == "-" || $6 == "" {
    if ($5 == "real") {
      print "line " NR ": real owner has no suite evidence" > "/dev/stderr"
      bad = 1
    }
  }
  $5 == "real" && $4 != "active" {
    print "line " NR ": real owner is not active" > "/dev/stderr"
    bad = 1
  }
  $5 == "bridge" && $7 !~ /owner|replace|remove|migrat/ {
    print "line " NR ": bridge has no replacement owner gate" > "/dev/stderr"
    bad = 1
  }
  {
    count++
  }
  END {
    if (count == 0 || bad) exit 1
  }
' "$map"

awk 'BEGIN { FS = sprintf("%c", 9) }
NR > 1 {
  n = split($3, owners, ";")
  for (i = 1; i <= n; i++) {
    owner = owners[i]
    gsub(/^ +| +$/, "", owner)
    if (owner ~ /^(skip:|deferred:|host-owned$)/) continue
    if (owner ~ /\/$/) {
      command = "test -d \"" owner "\""
    } else {
      command = "test -f \"" owner "\""
    }
    if (system(command) != 0) {
      print "missing semantic owner target: " owner > "/dev/stderr"
      bad = 1
    }
  }
  if ($5 == "real") {
    for (i = 1; i <= n; i++) {
      owner = owners[i]
      gsub(/^ +| +$/, "", owner)
      if (owner ~ /\.rs$/ && system("rg -q placeholder \"" owner "\"") == 0) {
        print "placeholder-only target cannot be real: " owner > "/dev/stderr"
        bad = 1
      }
    }
  }
} END { if (bad) exit 1 }' "$map"

echo "semantic ownership map: OK"
