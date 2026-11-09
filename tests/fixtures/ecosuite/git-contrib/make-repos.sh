#!/usr/bin/env bash
# rubash#2907 chain fixture (in-repo; first run as eco-2907).
# Build deterministic test repos for the git-contrib chain under $1 (repos root).
set -euo pipefail
ROOT="$1"
export GIT_AUTHOR_NAME='Eco Test' GIT_AUTHOR_EMAIL='eco@test.invalid'
export GIT_COMMITTER_NAME='Eco Test' GIT_COMMITTER_EMAIL='eco@test.invalid'
export GIT_AUTHOR_DATE='2026-11-01T12:00:00Z' GIT_COMMITTER_DATE='2026-11-01T12:00:00Z'
export GIT_CONFIG_NOSYSTEM=1
# Isolated HOME so no user gitconfig interferes.
export HOME="$ROOT/home"
mkdir -p "$HOME"

mk_base() { # $1 = dir ; creates main repo with branches/tags/remote
  local d="$1"
  rm -rf "$d"; mkdir -p "$d"; cd "$d"
  git init -q -b main .
  printf '# Demo\n' > README.md
  mkdir -p src docs
  printf 'fn main() {}\n' > src/main.rs
  printf '# Guide\n' > docs/guide.md
  git add -A
  git commit -qm 'initial commit'
  git tag v1.0
  git tag v1.1
  git branch feature/login
  git remote add origin "$ROOT/bare.git"
  git fetch -q origin 2>/dev/null || true
  git branch --set-upstream-to=origin/main main 2>/dev/null || true
}

# Bare remote (empty origin) so `git fetch` succeeds and upstream exists.
rm -rf "$ROOT/bare.git"; git init -q --bare "$ROOT/bare.git"
# Seed bare with main so ahead-tracking works: push from a temp clone.
TMP="$ROOT/_seed"; rm -rf "$TMP"; mkdir -p "$TMP"; cd "$TMP"
git init -q -b main .
printf '# Demo\n' > README.md
mkdir -p src docs
printf 'fn main() {}\n' > src/main.rs
printf '# Guide\n' > docs/guide.md
git add -A; git commit -qm 'initial commit'
git tag v1.0; git tag v1.1; git branch feature/login
git remote add origin "$ROOT/bare.git"
git push -q origin main feature/login v1.0 v1.1
git branch --set-upstream-to=origin/main main
git --git-dir "$ROOT/bare.git" symbolic-ref HEAD refs/heads/main
cd "$ROOT"
rm -rf "$TMP"

# main: the completion repo (clone of seeded bare, upstream set).
rm -rf "$ROOT/main"; git clone -q "$ROOT/bare.git" "$ROOT/main"; cd "$ROOT/main"
git checkout -q feature/login; git checkout -q main
git branch --set-upstream-to=origin/main main
git status --porcelain | grep . && { echo 'main repo not clean'; exit 1; } || true

# clean: fresh clone, clean tree.
rm -rf "$ROOT/clean"; git clone -q "$ROOT/bare.git" "$ROOT/clean"; cd "$ROOT/clean"
git branch --set-upstream-to=origin/main main

# dirty: unstaged modification only.
rm -rf "$ROOT/dirty"; git clone -q "$ROOT/bare.git" "$ROOT/dirty"; cd "$ROOT/dirty"
printf 'dirty line\n' >> README.md

# staged: staged new file, no unstaged change.
rm -rf "$ROOT/staged"; git clone -q "$ROOT/bare.git" "$ROOT/staged"; cd "$ROOT/staged"
printf 'staged\n' > staged.txt; git add staged.txt

# untracked: one untracked file.
rm -rf "$ROOT/untracked"; git clone -q "$ROOT/bare.git" "$ROOT/untracked"; cd "$ROOT/untracked"
printf 'untracked\n' > untracked.txt

# all: unstaged + staged + untracked + one stash entry.
rm -rf "$ROOT/all"; git clone -q "$ROOT/bare.git" "$ROOT/all"; cd "$ROOT/all"
printf 'stash me\n' > stash.txt; git add stash.txt; git stash -q -m 'eco stash' --include-untracked
printf 'dirty line\n' >> README.md
printf 'staged\n' > staged.txt; git add staged.txt
printf 'untracked\n' > untracked.txt

# detached: HEAD detached at the commit.
rm -rf "$ROOT/detached"; git clone -q "$ROOT/bare.git" "$ROOT/detached"; cd "$ROOT/detached"
git checkout -q --detach HEAD

# ahead: one local commit ahead of origin/main.
rm -rf "$ROOT/ahead"; git clone -q "$ROOT/bare.git" "$ROOT/ahead"; cd "$ROOT/ahead"
git branch --set-upstream-to=origin/main main
printf 'ahead\n' >> README.md; git add README.md; git commit -qm 'one ahead'

echo "repos built under $ROOT"
for d in main clean dirty staged untracked all detached ahead; do
  printf '%s: ' "$d"; git -C "$ROOT/$d" status --porcelain=v1 -b | head -3 | tr '\n' ';'; echo
done
