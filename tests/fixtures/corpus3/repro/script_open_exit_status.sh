#!/bin/sh
# rubash corpus3 lane: exit status when the script argument cannot be opened.
# GNU bash 5.3.0: -n missing -> 127; exec missing -> 127; directory -> 126
#   (shell.c open_shell_script: sh_exit((e==ENOENT)?EX_NOTFOUND:EX_NOINPUT),
#    shell.h:66 EX_NOTFOUND=127, shell.h:65 EX_NOINPUT=126)
# rubash (86818357): 1 / 1 / 1 (message text matches, status code does not).
SELF_DIR="$(cd "$(dirname "$0")" && pwd)"
"$RUBASH_OR_EMPTY" -n "$SELF_DIR/definitely-missing-corpus3.sh" 2>/dev/null
echo "missing -n rc=$?"
