#!/usr/bin/env bash
# ruby-build offline execution chain (rubash#2908, eco-asset census
# follow-up). The corpus copy at tests/regression/corpus/ruby-build/
# was previously only a parse/perf canary ("not executed" per
# tests/regression/corpus/PROVENANCE.txt). This fixture EXECUTES it
# end-to-end, offline, under the shell under test ("$BASH" is this
# shell; the corpus file is mode 644 and its shebang would resolve to
# the system shell, so it is always invoked through "$BASH"):
#
#   step 1  `ruby-build --definitions` over a fixture definition
#           registry (option parsing + definition listing);
#   step 2  `--resolve fake` picks the latest fake definition (the
#           step-1 listing feeds this choice);
#   step 3  install fake-1.0.0 into a scratch prefix: the definition
#           is evaluated, its tarball is fetched over a file:// URL
#           with the pinned curl client (real HEAD/GET flow, no
#           network), the SHA-256 in the definition is verified, the
#           tarball is extracted, before/after install hooks fire,
#           and the `copy` build step populates the prefix;
#   step 4  the installed payload is consumed: its note file is read
#           back and its tool is executed from the prefix (step-3
#           state feeds step 4), and the build path is asserted wiped;
#   step 5  the step-2 resolution is installed and its tool runs;
#   step 6  a definition with a wrong checksum must fail at the
#           checksum gate: nonzero rc, the mismatch line on stderr,
#           BUILD FAILED, and no prefix created;
#   step 7  the definition registry is listed again and counted —
#           the chain leaves it untouched.
#
# Determinism pins: RUBY_BUILD_BUILD_PATH and TMPDIR are fixture
# directories; the HTTP client is pinned to curl with --silent (the
# default curl progress meter prints speed/timing bytes); tarball
# checksums are computed at runtime and never printed; the one
# absolute path ruby-build prints (the "Installed ... to" prefix) is
# normalized to SCRATCH by sed. Everything else is literal output.
#
# Capture (run from a scratch copy of this directory WITH ruby-build,
# in the harness environment: env -i, PATH=/usr/bin:/bin, HOME=scratch):
#   env -i PATH=/usr/bin:/bin HOME=$PWD /path/to/gnu-bash-5.3.20 \
#     issue2908-ruby-build-exec.sh \
#     > issue2908-ruby-build-exec.gnu.out \
#     2> issue2908-ruby-build-exec.gnu.err
export RUBY_BUILD_DEFINITIONS="$PWD/defs"
export RUBY_BUILD_BUILD_PATH="$PWD/build"
export RUBY_BUILD_HTTP_CLIENT=curl
export RUBY_BUILD_CURL_OPTS="--silent"
export TMPDIR="$PWD/tmp"
mkdir -p defs pkgs tmp

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | { read -r h _; echo "$h"; }
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | { read -r h _; echo "$h"; }
  else
    openssl dgst -sha256 "$1" | sed 's/^.*= //'
  fi
}

for v in 1.0.0 2.0.0; do
  d="src/fake-$v"
  mkdir -p "$d/bin" "$d/share"
  printf '#!/bin/sh\necho "fake-tool %s works"\n' "$v" > "$d/bin/fake-tool"
  chmod +x "$d/bin/fake-tool"
  printf 'fake payload %s\n' "$v" > "$d/share/note.txt"
  tar czf "pkgs/fake-$v.tar.gz" -C src "fake-$v"
  sum="$(sha256_of "pkgs/fake-$v.tar.gz")"
  {
    echo 'before_install_package() { echo "HOOK before $1"; }'
    echo 'after_install_package() { echo "HOOK after $1"; }'
    echo "install_package \"fake-$v\" \"file://$PWD/pkgs/fake-$v.tar.gz#$sum\" copy"
  } > "defs/fake-$v"
done

d="src/broken-9.9.9"
mkdir -p "$d/bin"
printf '#!/bin/sh\necho broken\n' > "$d/bin/fake-tool"
chmod +x "$d/bin/fake-tool"
tar czf "pkgs/broken-9.9.9.tar.gz" -C src "broken-9.9.9"
zeros="0000000000000000000000000000000000000000000000000000000000000000"
echo "install_package \"broken-9.9.9\" \"file://$PWD/pkgs/broken-9.9.9.tar.gz#$zeros\" copy" \
  > "defs/broken-9.9.9"

echo "== definitions =="
"$BASH" ./ruby-build --definitions
echo "defs-rc=$?"

echo "== resolve =="
latest="$("$BASH" ./ruby-build --resolve fake)"
echo "resolved=$latest"

echo "== install fake-1.0.0 =="
"$BASH" ./ruby-build fake-1.0.0 installed1 2>install1.err | sed "s|$PWD|SCRATCH|g"
echo "install1-rc=${PIPESTATUS[0]}"
[ -s install1.err ] && echo "install1-stderr=nonempty" || echo "install1-stderr=empty"
cat installed1/share/note.txt
installed1/bin/fake-tool
echo "tool1-rc=$?"
[ -e build ] && echo "buildpath=kept" || echo "buildpath=wiped"

echo "== install resolved =="
"$BASH" ./ruby-build "$latest" installed2 2>install2.err | sed "s|$PWD|SCRATCH|g"
echo "install2-rc=${PIPESTATUS[0]}"
[ -s install2.err ] && echo "install2-stderr=nonempty" || echo "install2-stderr=empty"
installed2/bin/fake-tool

echo "== broken checksum =="
"$BASH" ./ruby-build broken-9.9.9 installed-bad >broken.out 2>broken.err
echo "broken-rc=$?"
sed "s|$PWD|SCRATCH|g" broken.out
echo "mismatch-lines=$(grep -c 'checksum mismatch: broken-9.9.9.tar.gz (file is corrupt)' broken.err)"
echo "buildfailed-lines=$(grep -c 'BUILD FAILED' broken.err)"
[ -e installed-bad ] && echo "badprefix=present" || echo "badprefix=absent"

echo "== final definitions count =="
echo "final-defs=$("$BASH" ./ruby-build --definitions | grep -c .)"
echo done
