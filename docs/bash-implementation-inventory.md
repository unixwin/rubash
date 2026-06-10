# Bash Implementation Inventory

This appendix assigns each implementation-shaped GNU Bash source file to a Rubash target module. It is an ownership map, not a claim that the C file has already been ported.

| GNU Bash file | Rubash target |
|---|---|
| `CWRU/misc/errlist.c` | `skip: build/support/example tool` |
| `CWRU/misc/hpux10-dlfcn.h` | `skip: build/support/example tool` |
| `CWRU/misc/open-files.c` | `skip: build/support/example tool` |
| `CWRU/misc/sigs.c` | `skip: build/support/example tool` |
| `CWRU/misc/sigstat.c` | `skip: build/support/example tool` |
| `alias.c` | `src/shell/alias.rs` |
| `alias.h` | `src/shell/alias.rs` |
| `array.c` | `src/shell/arrays/indexed.rs` |
| `array.h` | `src/shell/arrays/indexed.rs` |
| `array2.c` | `src/shell/arrays/indexed_extra.rs` |
| `arrayfunc.c` | `src/shell/arrays/functions.rs` |
| `arrayfunc.h` | `src/shell/arrays/functions.rs` |
| `assoc.c` | `src/shell/arrays/assoc.rs` |
| `assoc.h` | `src/shell/arrays/assoc.rs` |
| `bashansi.h` | `src/sys/compat.rs` |
| `bashhist.c` | `src/history/bashhist.rs` |
| `bashhist.h` | `src/history/bashhist.rs` |
| `bashintl.h` | `src/sys/compat.rs` |
| `bashjmp.h` | `src/sys/compat.rs` |
| `bashline.c` | `src/input/bashline.rs` |
| `bashline.h` | `src/input/bashline.rs` |
| `bashtypes.h` | `src/sys/compat.rs` |
| `bracecomp.c` | `src/expand/bracecomp.rs` |
| `braces.c` | `src/expand/braces.rs` |
| `builtins.h` | `src/sys/compat.rs` |
| `builtins/alias.def` | `src/builtins/alias.rs` |
| `builtins/bashgetopt.c` | `src/builtins/getopt.rs` |
| `builtins/bashgetopt.h` | `src/builtins/getopt.rs` |
| `builtins/bind.def` | `src/builtins/bind.rs` |
| `builtins/break.def` | `src/builtins/break.rs` |
| `builtins/builtin.def` | `src/builtins/builtin.rs` |
| `builtins/caller.def` | `src/builtins/caller.rs` |
| `builtins/cd.def` | `src/builtins/cd.rs` |
| `builtins/colon.def` | `src/builtins/colon.rs` |
| `builtins/command.def` | `src/builtins/command.rs` |
| `builtins/common.c` | `src/builtins/common.rs` |
| `builtins/common.h` | `src/builtins/common.rs` |
| `builtins/complete.def` | `src/builtins/complete.rs` |
| `builtins/declare.def` | `src/builtins/declare.rs` |
| `builtins/echo.def` | `src/builtins/echo.rs` |
| `builtins/enable.def` | `src/builtins/enable.rs` |
| `builtins/eval.def` | `src/builtins/eval.rs` |
| `builtins/evalfile.c` | `src/builtins/evalfile.rs` |
| `builtins/evalstring.c` | `src/builtins/evalstring.rs` |
| `builtins/exec.def` | `src/builtins/exec.rs` |
| `builtins/exit.def` | `src/builtins/exit.rs` |
| `builtins/fc.def` | `src/builtins/fc.rs` |
| `builtins/fg_bg.def` | `src/builtins/fg_bg.rs` |
| `builtins/gen-helpfiles.c` | `src/builtins/support.rs` |
| `builtins/getopt.c` | `src/builtins/getopt.rs` |
| `builtins/getopt.h` | `src/builtins/getopt.rs` |
| `builtins/getopts.def` | `src/builtins/getopts.rs` |
| `builtins/hash.def` | `src/builtins/hash.rs` |
| `builtins/help.def` | `src/builtins/help.rs` |
| `builtins/history.def` | `src/builtins/history.rs` |
| `builtins/jobs.def` | `src/builtins/jobs.rs` |
| `builtins/kill.def` | `src/builtins/kill.rs` |
| `builtins/let.def` | `src/builtins/let.rs` |
| `builtins/mapfile.def` | `src/builtins/mapfile.rs` |
| `builtins/mkbuiltins.c` | `src/builtins/support.rs` |
| `builtins/printf.def` | `src/builtins/printf.rs` |
| `builtins/psize.c` | `src/builtins/support.rs` |
| `builtins/pushd.def` | `src/builtins/pushd.rs` |
| `builtins/read.def` | `src/builtins/read.rs` |
| `builtins/reserved.def` | `src/builtins/reserved.rs` |
| `builtins/return.def` | `src/builtins/return.rs` |
| `builtins/set.def` | `src/builtins/set.rs` |
| `builtins/setattr.def` | `src/builtins/setattr.rs` |
| `builtins/shift.def` | `src/builtins/shift.rs` |
| `builtins/shopt.def` | `src/builtins/shopt.rs` |
| `builtins/source.def` | `src/builtins/source.rs` |
| `builtins/suspend.def` | `src/builtins/suspend.rs` |
| `builtins/test.def` | `src/builtins/test.rs` |
| `builtins/times.def` | `src/builtins/times.rs` |
| `builtins/trap.def` | `src/builtins/trap.rs` |
| `builtins/type.def` | `src/builtins/type.rs` |
| `builtins/ulimit.def` | `src/builtins/ulimit.rs` |
| `builtins/umask.def` | `src/builtins/umask.rs` |
| `builtins/wait.def` | `src/builtins/wait.rs` |
| `command.h` | `src/parser/ast.rs` |
| `config-bot.h` | `src/sys/compat.rs` |
| `config-top.h` | `src/sys/compat.rs` |
| `conftypes.h` | `src/sys/compat.rs` |
| `copy_cmd.c` | `src/parser/copy.rs` |
| `dispose_cmd.c` | `src/parser/dispose.rs` |
| `dispose_cmd.h` | `src/parser/dispose.rs` |
| `error.c` | `src/shell/error.rs` |
| `error.h` | `src/shell/error.rs` |
| `eval.c` | `src/executor/eval.rs` |
| `examples/loadables/accept.c` | `skip: build/support/example tool` |
| `examples/loadables/asort.c` | `skip: build/support/example tool` |
| `examples/loadables/basename.c` | `skip: build/support/example tool` |
| `examples/loadables/cat.c` | `skip: build/support/example tool` |
| `examples/loadables/chmod.c` | `skip: build/support/example tool` |
| `examples/loadables/csv.c` | `skip: build/support/example tool` |
| `examples/loadables/cut.c` | `skip: build/support/example tool` |
| `examples/loadables/dirname.c` | `skip: build/support/example tool` |
| `examples/loadables/dsv.c` | `skip: build/support/example tool` |
| `examples/loadables/fdflags.c` | `skip: build/support/example tool` |
| `examples/loadables/finfo.c` | `skip: build/support/example tool` |
| `examples/loadables/fltexpr.c` | `skip: build/support/example tool` |
| `examples/loadables/getconf.c` | `skip: build/support/example tool` |
| `examples/loadables/getconf.h` | `skip: build/support/example tool` |
| `examples/loadables/head.c` | `skip: build/support/example tool` |
| `examples/loadables/hello.c` | `skip: build/support/example tool` |
| `examples/loadables/id.c` | `skip: build/support/example tool` |
| `examples/loadables/kv.c` | `skip: build/support/example tool` |
| `examples/loadables/ln.c` | `skip: build/support/example tool` |
| `examples/loadables/loadables.h` | `skip: build/support/example tool` |
| `examples/loadables/logname.c` | `skip: build/support/example tool` |
| `examples/loadables/mkdir.c` | `skip: build/support/example tool` |
| `examples/loadables/mkfifo.c` | `skip: build/support/example tool` |
| `examples/loadables/mktemp.c` | `skip: build/support/example tool` |
| `examples/loadables/mypid.c` | `skip: build/support/example tool` |
| `examples/loadables/necho.c` | `skip: build/support/example tool` |
| `examples/loadables/ocut.c` | `skip: build/support/example tool` |
| `examples/loadables/pathchk.c` | `skip: build/support/example tool` |
| `examples/loadables/perl/bperl.c` | `skip: build/support/example tool` |
| `examples/loadables/perl/iperl.c` | `skip: build/support/example tool` |
| `examples/loadables/print.c` | `skip: build/support/example tool` |
| `examples/loadables/printenv.c` | `skip: build/support/example tool` |
| `examples/loadables/push.c` | `skip: build/support/example tool` |
| `examples/loadables/realpath.c` | `skip: build/support/example tool` |
| `examples/loadables/rm.c` | `skip: build/support/example tool` |
| `examples/loadables/rmdir.c` | `skip: build/support/example tool` |
| `examples/loadables/seq.c` | `skip: build/support/example tool` |
| `examples/loadables/setpgid.c` | `skip: build/support/example tool` |
| `examples/loadables/sleep.c` | `skip: build/support/example tool` |
| `examples/loadables/stat.c` | `skip: build/support/example tool` |
| `examples/loadables/strftime.c` | `skip: build/support/example tool` |
| `examples/loadables/strptime.c` | `skip: build/support/example tool` |
| `examples/loadables/sync.c` | `skip: build/support/example tool` |
| `examples/loadables/tee.c` | `skip: build/support/example tool` |
| `examples/loadables/template.c` | `skip: build/support/example tool` |
| `examples/loadables/truefalse.c` | `skip: build/support/example tool` |
| `examples/loadables/tty.c` | `skip: build/support/example tool` |
| `examples/loadables/uname.c` | `skip: build/support/example tool` |
| `examples/loadables/unlink.c` | `skip: build/support/example tool` |
| `examples/loadables/whoami.c` | `skip: build/support/example tool` |
| `execute_cmd.c` | `src/executor/command.rs` |
| `execute_cmd.h` | `src/executor/command.rs` |
| `expr.c` | `src/expand/arithmetic.rs` |
| `externs.h` | `src/sys/compat.rs` |
| `findcmd.c` | `src/executor/path.rs` |
| `findcmd.h` | `src/executor/path.rs` |
| `flags.c` | `src/shell/options.rs` |
| `flags.h` | `src/shell/options.rs` |
| `general.c` | `src/shell/general.rs` |
| `general.h` | `src/shell/general.rs` |
| `hashcmd.c` | `src/executor/hash.rs` |
| `hashcmd.h` | `src/executor/hash.rs` |
| `hashlib.c` | `src/executor/hashlib.rs` |
| `hashlib.h` | `src/executor/hashlib.rs` |
| `include/ansi_stdlib.h` | `src/sys/include.rs` |
| `include/chartypes.h` | `src/sys/include.rs` |
| `include/filecntl.h` | `src/sys/include.rs` |
| `include/gettext.h` | `src/sys/include.rs` |
| `include/intprops-internal.h` | `src/sys/include.rs` |
| `include/maxpath.h` | `src/sys/include.rs` |
| `include/memalloc.h` | `src/sys/include.rs` |
| `include/ocache.h` | `src/sys/include.rs` |
| `include/posixdir.h` | `src/sys/include.rs` |
| `include/posixjmp.h` | `src/sys/include.rs` |
| `include/posixselect.h` | `src/sys/include.rs` |
| `include/posixstat.h` | `src/sys/include.rs` |
| `include/posixtime.h` | `src/sys/include.rs` |
| `include/posixwait.h` | `src/sys/include.rs` |
| `include/shmbchar.h` | `src/sys/include.rs` |
| `include/shmbutil.h` | `src/sys/include.rs` |
| `include/shtty.h` | `src/sys/include.rs` |
| `include/stat-time.h` | `src/sys/include.rs` |
| `include/stdc.h` | `src/sys/include.rs` |
| `include/stdckdint.in.h` | `src/sys/include.rs` |
| `include/systimes.h` | `src/sys/include.rs` |
| `include/timer.h` | `src/sys/include.rs` |
| `include/typemax.h` | `src/sys/include.rs` |
| `include/unionwait.h` | `src/sys/include.rs` |
| `include/unlocked-io.h` | `src/sys/include.rs` |
| `input.c` | `src/input/input.rs` |
| `input.h` | `src/input/input.rs` |
| `jobs.c` | `src/jobs/jobs.rs` |
| `jobs.h` | `src/jobs/jobs.rs` |
| `lib/glob/collsyms.h` | `src/expand/glob/collsyms.rs` |
| `lib/glob/glob.c` | `src/expand/glob/glob.rs` |
| `lib/glob/glob.h` | `src/expand/glob/glob.rs` |
| `lib/glob/glob_loop.c` | `src/expand/glob/glob_loop.rs` |
| `lib/glob/gm_loop.c` | `src/expand/glob/gm_loop.rs` |
| `lib/glob/gmisc.c` | `src/expand/glob/gmisc.rs` |
| `lib/glob/sm_loop.c` | `src/expand/glob/sm_loop.rs` |
| `lib/glob/smatch.c` | `src/expand/glob/smatch.rs` |
| `lib/glob/strmatch.c` | `src/expand/glob/strmatch.rs` |
| `lib/glob/strmatch.h` | `src/expand/glob/strmatch.rs` |
| `lib/glob/xmbsrtowcs.c` | `src/expand/glob/xmbsrtowcs.rs` |
| `lib/intl/arg-nonnull.h` | `src/locale/intl/arg-nonnull.rs` |
| `lib/intl/attribute.h` | `src/locale/intl/attribute.rs` |
| `lib/intl/bindtextdom.c` | `src/locale/intl/bindtextdom.rs` |
| `lib/intl/dcgettext.c` | `src/locale/intl/dcgettext.rs` |
| `lib/intl/dcigettext.c` | `src/locale/intl/dcigettext.rs` |
| `lib/intl/dcngettext.c` | `src/locale/intl/dcngettext.rs` |
| `lib/intl/dgettext.c` | `src/locale/intl/dgettext.rs` |
| `lib/intl/dngettext.c` | `src/locale/intl/dngettext.rs` |
| `lib/intl/eval-plural.h` | `src/locale/intl/eval-plural.rs` |
| `lib/intl/explodename.c` | `src/locale/intl/explodename.rs` |
| `lib/intl/export.h` | `src/locale/intl/export.rs` |
| `lib/intl/filename.h` | `src/locale/intl/filename.rs` |
| `lib/intl/finddomain.c` | `src/locale/intl/finddomain.rs` |
| `lib/intl/flexmember.h` | `src/locale/intl/flexmember.rs` |
| `lib/intl/gettext.c` | `src/locale/intl/gettext.rs` |
| `lib/intl/gettextP.h` | `src/locale/intl/gettextP.rs` |
| `lib/intl/gmo.h` | `src/locale/intl/gmo.rs` |
| `lib/intl/hash-string.c` | `src/locale/intl/hash-string.rs` |
| `lib/intl/hash-string.h` | `src/locale/intl/hash-string.rs` |
| `lib/intl/intl-compat.c` | `src/locale/intl/intl-compat.rs` |
| `lib/intl/intl-exports.c` | `src/locale/intl/intl-exports.rs` |
| `lib/intl/l10nflist.c` | `src/locale/intl/l10nflist.rs` |
| `lib/intl/langprefs.c` | `src/locale/intl/langprefs.rs` |
| `lib/intl/libgnuintl.in.h` | `src/locale/intl/libgnuintl.in.rs` |
| `lib/intl/loadinfo.h` | `src/locale/intl/loadinfo.rs` |
| `lib/intl/loadmsgcat.c` | `src/locale/intl/loadmsgcat.rs` |
| `lib/intl/localcharset.c` | `src/locale/intl/localcharset.rs` |
| `lib/intl/localcharset.h` | `src/locale/intl/localcharset.rs` |
| `lib/intl/localealias.c` | `src/locale/intl/localealias.rs` |
| `lib/intl/localename-table.c` | `src/locale/intl/localename-table.rs` |
| `lib/intl/localename-table.in.h` | `src/locale/intl/localename-table.in.rs` |
| `lib/intl/localename.c` | `src/locale/intl/localename.rs` |
| `lib/intl/lock.c` | `src/locale/intl/lock.rs` |
| `lib/intl/lock.h` | `src/locale/intl/lock.rs` |
| `lib/intl/log.c` | `src/locale/intl/log.rs` |
| `lib/intl/ngettext.c` | `src/locale/intl/ngettext.rs` |
| `lib/intl/os2compat.c` | `src/locale/intl/os2compat.rs` |
| `lib/intl/os2compat.h` | `src/locale/intl/os2compat.rs` |
| `lib/intl/osdep.c` | `src/locale/intl/osdep.rs` |
| `lib/intl/plural-exp.c` | `src/locale/intl/plural-exp.rs` |
| `lib/intl/plural-exp.h` | `src/locale/intl/plural-exp.rs` |
| `lib/intl/plural.c` | `src/locale/intl/plural.rs` |
| `lib/intl/plural.h` | `src/locale/intl/plural.rs` |
| `lib/intl/plural.y` | `src/locale/intl/plural.rs` |
| `lib/intl/printf-args.c` | `src/locale/intl/printf-args.rs` |
| `lib/intl/printf-args.h` | `src/locale/intl/printf-args.rs` |
| `lib/intl/printf-parse.c` | `src/locale/intl/printf-parse.rs` |
| `lib/intl/printf-parse.h` | `src/locale/intl/printf-parse.rs` |
| `lib/intl/printf.c` | `src/locale/intl/printf.rs` |
| `lib/intl/relocatable.c` | `src/locale/intl/relocatable.rs` |
| `lib/intl/relocatable.h` | `src/locale/intl/relocatable.rs` |
| `lib/intl/setlocale-lock.c` | `src/locale/intl/setlocale-lock.rs` |
| `lib/intl/setlocale.c` | `src/locale/intl/setlocale.rs` |
| `lib/intl/setlocale_null.c` | `src/locale/intl/setlocale_null.rs` |
| `lib/intl/setlocale_null.h` | `src/locale/intl/setlocale_null.rs` |
| `lib/intl/textdomain.c` | `src/locale/intl/textdomain.rs` |
| `lib/intl/thread-optim.h` | `src/locale/intl/thread-optim.rs` |
| `lib/intl/threadlib.c` | `src/locale/intl/threadlib.rs` |
| `lib/intl/tsearch.c` | `src/locale/intl/tsearch.rs` |
| `lib/intl/tsearch.h` | `src/locale/intl/tsearch.rs` |
| `lib/intl/vasnprintf.c` | `src/locale/intl/vasnprintf.rs` |
| `lib/intl/vasnprintf.h` | `src/locale/intl/vasnprintf.rs` |
| `lib/intl/vasnwprintf.h` | `src/locale/intl/vasnwprintf.rs` |
| `lib/intl/verify.h` | `src/locale/intl/verify.rs` |
| `lib/intl/version.c` | `src/locale/intl/version.rs` |
| `lib/intl/wprintf-parse.h` | `src/locale/intl/wprintf-parse.rs` |
| `lib/intl/xsize.c` | `src/locale/intl/xsize.rs` |
| `lib/intl/xsize.h` | `src/locale/intl/xsize.rs` |
| `lib/malloc/alloca.c` | `skip: Rust allocator/std` |
| `lib/malloc/getpagesize.h` | `skip: Rust allocator/std` |
| `lib/malloc/imalloc.h` | `skip: Rust allocator/std` |
| `lib/malloc/malloc.c` | `skip: Rust allocator/std` |
| `lib/malloc/mstats.h` | `skip: Rust allocator/std` |
| `lib/malloc/sbrk.c` | `skip: Rust allocator/std` |
| `lib/malloc/shmalloc.h` | `skip: Rust allocator/std` |
| `lib/malloc/stats.c` | `skip: Rust allocator/std` |
| `lib/malloc/stub.c` | `skip: Rust allocator/std` |
| `lib/malloc/table.c` | `skip: Rust allocator/std` |
| `lib/malloc/table.h` | `skip: Rust allocator/std` |
| `lib/malloc/trace.c` | `skip: Rust allocator/std` |
| `lib/malloc/watch.c` | `skip: Rust allocator/std` |
| `lib/malloc/watch.h` | `skip: Rust allocator/std` |
| `lib/malloc/xmalloc.c` | `skip: Rust allocator/std` |
| `lib/readline/ansi_stdlib.h` | `src/input/readline/ansi_stdlib.rs` |
| `lib/readline/bind.c` | `src/input/readline/bind.rs` |
| `lib/readline/callback.c` | `src/input/readline/callback.rs` |
| `lib/readline/chardefs.h` | `src/input/readline/chardefs.rs` |
| `lib/readline/colors.c` | `src/input/readline/colors.rs` |
| `lib/readline/colors.h` | `src/input/readline/colors.rs` |
| `lib/readline/compat.c` | `src/input/readline/compat.rs` |
| `lib/readline/complete.c` | `src/input/readline/complete.rs` |
| `lib/readline/display.c` | `src/input/readline/display.rs` |
| `lib/readline/emacs_keymap.c` | `src/input/readline/emacs_keymap.rs` |
| `lib/readline/examples/excallback.c` | `src/input/readline/excallback.rs` |
| `lib/readline/examples/fileman.c` | `src/input/readline/fileman.rs` |
| `lib/readline/examples/histexamp.c` | `src/input/readline/histexamp.rs` |
| `lib/readline/examples/manexamp.c` | `src/input/readline/manexamp.rs` |
| `lib/readline/examples/rl-callbacktest.c` | `src/input/readline/rl-callbacktest.rs` |
| `lib/readline/examples/rl.c` | `src/input/readline/rl.rs` |
| `lib/readline/examples/rlcat.c` | `src/input/readline/rlcat.rs` |
| `lib/readline/examples/rltest.c` | `src/input/readline/rltest.rs` |
| `lib/readline/funmap.c` | `src/input/readline/funmap.rs` |
| `lib/readline/histexpand.c` | `src/input/readline/histexpand.rs` |
| `lib/readline/histfile.c` | `src/input/readline/histfile.rs` |
| `lib/readline/histlib.h` | `src/input/readline/histlib.rs` |
| `lib/readline/history.c` | `src/input/readline/history.rs` |
| `lib/readline/history.h` | `src/input/readline/history.rs` |
| `lib/readline/histsearch.c` | `src/input/readline/histsearch.rs` |
| `lib/readline/input.c` | `src/input/readline/input.rs` |
| `lib/readline/isearch.c` | `src/input/readline/isearch.rs` |
| `lib/readline/keymaps.c` | `src/input/readline/keymaps.rs` |
| `lib/readline/keymaps.h` | `src/input/readline/keymaps.rs` |
| `lib/readline/kill.c` | `src/input/readline/kill.rs` |
| `lib/readline/macro.c` | `src/input/readline/macro.rs` |
| `lib/readline/mbutil.c` | `src/input/readline/mbutil.rs` |
| `lib/readline/misc.c` | `src/input/readline/misc.rs` |
| `lib/readline/nls.c` | `src/input/readline/nls.rs` |
| `lib/readline/parens.c` | `src/input/readline/parens.rs` |
| `lib/readline/parse-colors.c` | `src/input/readline/parse-colors.rs` |
| `lib/readline/parse-colors.h` | `src/input/readline/parse-colors.rs` |
| `lib/readline/posixdir.h` | `src/input/readline/posixdir.rs` |
| `lib/readline/posixjmp.h` | `src/input/readline/posixjmp.rs` |
| `lib/readline/posixselect.h` | `src/input/readline/posixselect.rs` |
| `lib/readline/posixstat.h` | `src/input/readline/posixstat.rs` |
| `lib/readline/posixtime.h` | `src/input/readline/posixtime.rs` |
| `lib/readline/readline.c` | `src/input/readline/readline.rs` |
| `lib/readline/readline.h` | `src/input/readline/readline.rs` |
| `lib/readline/rlconf.h` | `src/input/readline/rlconf.rs` |
| `lib/readline/rldefs.h` | `src/input/readline/rldefs.rs` |
| `lib/readline/rlmbutil.h` | `src/input/readline/rlmbutil.rs` |
| `lib/readline/rlprivate.h` | `src/input/readline/rlprivate.rs` |
| `lib/readline/rlshell.h` | `src/input/readline/rlshell.rs` |
| `lib/readline/rlstdc.h` | `src/input/readline/rlstdc.rs` |
| `lib/readline/rltty.c` | `src/input/readline/rltty.rs` |
| `lib/readline/rltty.h` | `src/input/readline/rltty.rs` |
| `lib/readline/rltypedefs.h` | `src/input/readline/rltypedefs.rs` |
| `lib/readline/rlwinsize.h` | `src/input/readline/rlwinsize.rs` |
| `lib/readline/savestring.c` | `src/input/readline/savestring.rs` |
| `lib/readline/search.c` | `src/input/readline/search.rs` |
| `lib/readline/shell.c` | `src/input/readline/shell.rs` |
| `lib/readline/signals.c` | `src/input/readline/signals.rs` |
| `lib/readline/tcap.h` | `src/input/readline/tcap.rs` |
| `lib/readline/terminal.c` | `src/input/readline/terminal.rs` |
| `lib/readline/text.c` | `src/input/readline/text.rs` |
| `lib/readline/tilde.c` | `src/input/readline/tilde.rs` |
| `lib/readline/tilde.h` | `src/input/readline/tilde.rs` |
| `lib/readline/undo.c` | `src/input/readline/undo.rs` |
| `lib/readline/util.c` | `src/input/readline/util.rs` |
| `lib/readline/vi_keymap.c` | `src/input/readline/vi_keymap.rs` |
| `lib/readline/vi_mode.c` | `src/input/readline/vi_mode.rs` |
| `lib/readline/xfree.c` | `src/input/readline/xfree.rs` |
| `lib/readline/xmalloc.c` | `src/input/readline/xmalloc.rs` |
| `lib/readline/xmalloc.h` | `src/input/readline/xmalloc.rs` |
| `lib/sh/anonfile.c` | `src/sys/sh/anonfile.rs` |
| `lib/sh/casemod.c` | `src/sys/sh/casemod.rs` |
| `lib/sh/clktck.c` | `src/sys/sh/clktck.rs` |
| `lib/sh/clock.c` | `src/sys/sh/clock.rs` |
| `lib/sh/compat.c` | `src/sys/sh/compat.rs` |
| `lib/sh/dprintf.c` | `src/sys/sh/dprintf.rs` |
| `lib/sh/eaccess.c` | `src/sys/sh/eaccess.rs` |
| `lib/sh/fmtullong.c` | `src/sys/sh/fmtullong.rs` |
| `lib/sh/fmtulong.c` | `src/sys/sh/fmtulong.rs` |
| `lib/sh/fmtumax.c` | `src/sys/sh/fmtumax.rs` |
| `lib/sh/fnxform.c` | `src/sys/sh/fnxform.rs` |
| `lib/sh/fpurge.c` | `src/sys/sh/fpurge.rs` |
| `lib/sh/getcwd.c` | `src/sys/sh/getcwd.rs` |
| `lib/sh/getenv.c` | `src/sys/sh/getenv.rs` |
| `lib/sh/gettimeofday.c` | `src/sys/sh/gettimeofday.rs` |
| `lib/sh/inet_aton.c` | `src/sys/sh/inet_aton.rs` |
| `lib/sh/input_avail.c` | `src/sys/sh/input_avail.rs` |
| `lib/sh/itos.c` | `src/sys/sh/itos.rs` |
| `lib/sh/mailstat.c` | `src/sys/sh/mailstat.rs` |
| `lib/sh/makepath.c` | `src/sys/sh/makepath.rs` |
| `lib/sh/mbscasecmp.c` | `src/sys/sh/mbscasecmp.rs` |
| `lib/sh/mbschr.c` | `src/sys/sh/mbschr.rs` |
| `lib/sh/mbscmp.c` | `src/sys/sh/mbscmp.rs` |
| `lib/sh/mbsncmp.c` | `src/sys/sh/mbsncmp.rs` |
| `lib/sh/memset.c` | `src/sys/sh/memset.rs` |
| `lib/sh/mktime.c` | `src/sys/sh/mktime.rs` |
| `lib/sh/netconn.c` | `src/sys/sh/netconn.rs` |
| `lib/sh/netopen.c` | `src/sys/sh/netopen.rs` |
| `lib/sh/oslib.c` | `src/sys/sh/oslib.rs` |
| `lib/sh/pathcanon.c` | `src/sys/sh/pathcanon.rs` |
| `lib/sh/pathphys.c` | `src/sys/sh/pathphys.rs` |
| `lib/sh/random.c` | `src/sys/sh/random.rs` |
| `lib/sh/reallocarray.c` | `src/sys/sh/reallocarray.rs` |
| `lib/sh/rename.c` | `src/sys/sh/rename.rs` |
| `lib/sh/setlinebuf.c` | `src/sys/sh/setlinebuf.rs` |
| `lib/sh/shmatch.c` | `src/sys/sh/shmatch.rs` |
| `lib/sh/shmbchar.c` | `src/sys/sh/shmbchar.rs` |
| `lib/sh/shquote.c` | `src/sys/sh/shquote.rs` |
| `lib/sh/shtty.c` | `src/sys/sh/shtty.rs` |
| `lib/sh/snprintf.c` | `src/sys/sh/snprintf.rs` |
| `lib/sh/spell.c` | `src/sys/sh/spell.rs` |
| `lib/sh/strcasecmp.c` | `src/sys/sh/strcasecmp.rs` |
| `lib/sh/strcasestr.c` | `src/sys/sh/strcasestr.rs` |
| `lib/sh/strchrnul.c` | `src/sys/sh/strchrnul.rs` |
| `lib/sh/strdup.c` | `src/sys/sh/strdup.rs` |
| `lib/sh/strerror.c` | `src/sys/sh/strerror.rs` |
| `lib/sh/strftime.c` | `src/sys/sh/strftime.rs` |
| `lib/sh/stringlist.c` | `src/sys/sh/stringlist.rs` |
| `lib/sh/stringvec.c` | `src/sys/sh/stringvec.rs` |
| `lib/sh/strlcpy.c` | `src/sys/sh/strlcpy.rs` |
| `lib/sh/strnlen.c` | `src/sys/sh/strnlen.rs` |
| `lib/sh/strpbrk.c` | `src/sys/sh/strpbrk.rs` |
| `lib/sh/strscpy.c` | `src/sys/sh/strscpy.rs` |
| `lib/sh/strstr.c` | `src/sys/sh/strstr.rs` |
| `lib/sh/strtod.c` | `src/sys/sh/strtod.rs` |
| `lib/sh/strtoimax.c` | `src/sys/sh/strtoimax.rs` |
| `lib/sh/strtol.c` | `src/sys/sh/strtol.rs` |
| `lib/sh/strtoll.c` | `src/sys/sh/strtoll.rs` |
| `lib/sh/strtoul.c` | `src/sys/sh/strtoul.rs` |
| `lib/sh/strtoull.c` | `src/sys/sh/strtoull.rs` |
| `lib/sh/strtoumax.c` | `src/sys/sh/strtoumax.rs` |
| `lib/sh/strtrans.c` | `src/sys/sh/strtrans.rs` |
| `lib/sh/strvis.c` | `src/sys/sh/strvis.rs` |
| `lib/sh/timers.c` | `src/sys/sh/timers.rs` |
| `lib/sh/times.c` | `src/sys/sh/times.rs` |
| `lib/sh/timeval.c` | `src/sys/sh/timeval.rs` |
| `lib/sh/tmpfile.c` | `src/sys/sh/tmpfile.rs` |
| `lib/sh/uconvert.c` | `src/sys/sh/uconvert.rs` |
| `lib/sh/ufuncs.c` | `src/sys/sh/ufuncs.rs` |
| `lib/sh/unicode.c` | `src/sys/sh/unicode.rs` |
| `lib/sh/utf8.c` | `src/sys/sh/utf8.rs` |
| `lib/sh/vprint.c` | `src/sys/sh/vprint.rs` |
| `lib/sh/wcsdup.c` | `src/sys/sh/wcsdup.rs` |
| `lib/sh/wcsnwidth.c` | `src/sys/sh/wcsnwidth.rs` |
| `lib/sh/wcswidth.c` | `src/sys/sh/wcswidth.rs` |
| `lib/sh/winsize.c` | `src/sys/sh/winsize.rs` |
| `lib/sh/zcatfd.c` | `src/sys/sh/zcatfd.rs` |
| `lib/sh/zgetline.c` | `src/sys/sh/zgetline.rs` |
| `lib/sh/zmapfd.c` | `src/sys/sh/zmapfd.rs` |
| `lib/sh/zread.c` | `src/sys/sh/zread.rs` |
| `lib/sh/zwrite.c` | `src/sys/sh/zwrite.rs` |
| `lib/termcap/ltcap.h` | `src/input/termcap.rs` |
| `lib/termcap/termcap.c` | `src/input/termcap.rs` |
| `lib/termcap/termcap.h` | `src/input/termcap.rs` |
| `lib/termcap/tparam.c` | `src/input/termcap.rs` |
| `lib/termcap/version.c` | `src/input/termcap.rs` |
| `lib/tilde/shell.c` | `src/expand/tilde/shell.rs` |
| `lib/tilde/tilde.c` | `src/expand/tilde/tilde.rs` |
| `lib/tilde/tilde.h` | `src/expand/tilde/tilde.rs` |
| `list.c` | `src/shell/list.rs` |
| `locale.c` | `src/locale/mod.rs` |
| `mailcheck.c` | `src/shell/mailcheck.rs` |
| `mailcheck.h` | `src/shell/mailcheck.rs` |
| `make_cmd.c` | `src/parser/make.rs` |
| `make_cmd.h` | `src/parser/make.rs` |
| `mksyntax.c` | `src/lexer/syntax_table.rs` |
| `nojobs.c` | `src/jobs/nojobs.rs` |
| `parse.y` | `src/parser/grammar.rs` |
| `parser.h` | `src/parser/mod.rs` |
| `patchlevel.h` | `src/shell/version.rs` |
| `pathexp.c` | `src/expand/pathname.rs` |
| `pathexp.h` | `src/expand/pathname.rs` |
| `pcomplete.c` | `src/complete/pcomplete.rs` |
| `pcomplete.h` | `src/complete/pcomplete.rs` |
| `pcomplib.c` | `src/complete/pcomplib.rs` |
| `print_cmd.c` | `src/parser/print.rs` |
| `quit.h` | `src/shell/quit.rs` |
| `redir.c` | `src/executor/redirection.rs` |
| `redir.h` | `src/executor/redirection.rs` |
| `shell.c` | `src/shell/runtime.rs` |
| `shell.h` | `src/shell/runtime.rs` |
| `sig.c` | `src/jobs/signals.rs` |
| `sig.h` | `src/jobs/signals.rs` |
| `siglist.c` | `src/jobs/siglist.rs` |
| `siglist.h` | `src/jobs/siglist.rs` |
| `stringlib.c` | `src/sys/stringlib.rs` |
| `subst.c` | `src/expand/word.rs` |
| `subst.h` | `src/expand/mod.rs` |
| `support/bashversion.c` | `skip: build/support/example tool` |
| `support/man2html.c` | `skip: build/support/example tool` |
| `support/mksignames.c` | `skip: build/support/example tool` |
| `support/printenv.c` | `skip: build/support/example tool` |
| `support/recho.c` | `skip: build/support/example tool` |
| `support/siglen.c` | `skip: build/support/example tool` |
| `support/signames.c` | `skip: build/support/example tool` |
| `support/xcase.c` | `skip: build/support/example tool` |
| `support/zecho.c` | `skip: build/support/example tool` |
| `syntax.h` | `src/lexer/syntax.rs` |
| `test.c` | `src/builtins/test.rs` |
| `test.h` | `src/builtins/test.rs` |
| `trap.c` | `src/jobs/trap.rs` |
| `trap.h` | `src/jobs/trap.rs` |
| `unwind_prot.c` | `src/shell/unwind.rs` |
| `unwind_prot.h` | `src/shell/unwind.rs` |
| `variables.c` | `src/shell/variables.rs` |
| `variables.h` | `src/shell/variables.rs` |
| `version.c` | `src/shell/version.rs` |
| `xmalloc.c` | `skip: Rust allocator/std` |
| `xmalloc.h` | `skip: Rust allocator/std` |
| `y.tab.c` | `skip: generated parser artifact` |
| `y.tab.h` | `skip: generated parser artifact` |
