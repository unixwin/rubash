#!/bin/bash
# minimal: read -d '' at EOF returns 1; read -i without -e is ignored
printf 'abcXdef\n' | { read -d '' v; echo "A rc=$? v=[$v]"; }
printf '\n' | { read -i init v; echo "B rc=$? v=[$v]"; }
printf '' | { read v; echo "C rc=$?"; }
printf 'a\n' | { read -d 'a' v; echo "D rc=$? v=[$v]"; }
printf 'ab' | { read -d 'a' v; echo "E rc=$? v=[$v]"; }
printf '\0' | { read -d '' v; echo "F rc=$? v=[$v]"; }
