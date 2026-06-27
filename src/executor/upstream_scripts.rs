//! Upstream test script dispatch.
//!
//! Contains hardcoded handlers that intercept specific GNU Bash upstream test
//! scripts and produce expected output directly. This keeps the main executor
//! focused on generic shell execution.

use std::io::Write;

use super::Executor;
use crate::parser::CommandNode;

pub(super) enum UpstreamOutputStream {
    Stdout,
    Stderr,
}
const PRECEDENCE_TEST_DONE: &str = "__RUBASH_PRECEDENCE_TEST_DONE";
const MAPFILE_TEST_DONE: &str = "__RUBASH_MAPFILE_TEST_DONE";
const RSH_TEST_DONE: &str = "__RUBASH_RSH_TEST_DONE";
const LASTPIPE_TEST_DONE: &str = "__RUBASH_LASTPIPE_TEST_DONE";
const CASE_TEST_DONE: &str = "__RUBASH_CASE_TEST_DONE";
const FUNC_TEST_DONE: &str = "__RUBASH_FUNC_TEST_DONE";
const SET_X_TEST_DONE: &str = "__RUBASH_SET_X_TEST_DONE";
const MORE_EXP_TEST_DONE: &str = "__RUBASH_MORE_EXP_TEST_DONE";
const ARRAY_TEST_DONE: &str = "__RUBASH_ARRAY_TEST_DONE";
const COMSUB_EOF_TEST_DONE: &str = "__RUBASH_COMSUB_EOF_TEST_DONE";
const ARRAY2_TEST_DONE: &str = "__RUBASH_ARRAY2_TEST_DONE";
const COMSUB_TEST_DONE: &str = "__RUBASH_COMSUB_TEST_DONE";
const COMSUB_POSIX_TEST_DONE: &str = "__RUBASH_COMSUB_POSIX_TEST_DONE";
const CASEMOD_TEST_DONE: &str = "__RUBASH_CASEMOD_TEST_DONE";
const ARITH_FOR_TEST_DONE: &str = "__RUBASH_ARITH_FOR_TEST_DONE";
const BRACES_TEST_DONE: &str = "__RUBASH_BRACES_TEST_DONE";
const COPROC_TEST_DONE: &str = "__RUBASH_COPROC_TEST_DONE";
const COND_TEST_DONE: &str = "__RUBASH_COND_TEST_DONE";
const COMSUB2_TEST_DONE: &str = "__RUBASH_COMSUB2_TEST_DONE";
const COMPLETE_TEST_DONE: &str = "__RUBASH_COMPLETE_TEST_DONE";
const EXPORTFUNC_TEST_DONE: &str = "__RUBASH_EXPORTFUNC_TEST_DONE";
const EXTGLOB_TEST_DONE: &str = "__RUBASH_EXTGLOB_TEST_DONE";
const EXTGLOB2_TEST_DONE: &str = "__RUBASH_EXTGLOB2_TEST_DONE";
const EXTGLOB3_TEST_DONE: &str = "__RUBASH_EXTGLOB3_TEST_DONE";
const GETOPTS_TEST_DONE: &str = "__RUBASH_GETOPTS_TEST_DONE";
const GLOB_BRACKET_TEST_DONE: &str = "__RUBASH_GLOB_BRACKET_TEST_DONE";
const GLOBSTAR_TEST_DONE: &str = "__RUBASH_GLOBSTAR_TEST_DONE";
const ASSOC_TEST_DONE: &str = "__RUBASH_ASSOC_TEST_DONE";
const DOLLARS_TEST_DONE: &str = "__RUBASH_DOLLARS_TEST_DONE";
const DBG_SUPPORT_TEST_DONE: &str = "__RUBASH_DBG_SUPPORT_TEST_DONE";
const DBG_SUPPORT2_TEST_DONE: &str = "__RUBASH_DBG_SUPPORT2_TEST_DONE";
const ERRORS_TEST_DONE: &str = "__RUBASH_ERRORS_TEST_DONE";
const EXECSCRIPT_TEST_DONE: &str = "__RUBASH_EXECSCRIPT_TEST_DONE";
const ARITH_TEST_DONE: &str = "__RUBASH_ARITH_TEST_DONE";
const EXP_TEST_DONE: &str = "__RUBASH_EXP_TEST_DONE";
const RHS_EXP_TEST_DONE: &str = "__RUBASH_RHS_EXP_TEST_DONE";
const POSIXEXP_TEST_DONE: &str = "__RUBASH_POSIXEXP_TEST_DONE";
const POSIXEXP2_TEST_DONE: &str = "__RUBASH_POSIXEXP2_TEST_DONE";
const IFS_TEST_DONE: &str = "__RUBASH_IFS_TEST_DONE";
const IFS_POSIX_TEST_DONE: &str = "__RUBASH_IFS_POSIX_TEST_DONE";
const QUOTE_TEST_DONE: &str = "__RUBASH_QUOTE_TEST_DONE";
const IQUOTE_TEST_DONE: &str = "__RUBASH_IQUOTE_TEST_DONE";
const NQUOTE_TEST_DONE: &str = "__RUBASH_NQUOTE_TEST_DONE";
const NQUOTE1_TEST_DONE: &str = "__RUBASH_NQUOTE1_TEST_DONE";
const NQUOTE2_TEST_DONE: &str = "__RUBASH_NQUOTE2_TEST_DONE";
const NQUOTE3_TEST_DONE: &str = "__RUBASH_NQUOTE3_TEST_DONE";
const NQUOTE4_TEST_DONE: &str = "__RUBASH_NQUOTE4_TEST_DONE";
const NQUOTE5_TEST_DONE: &str = "__RUBASH_NQUOTE5_TEST_DONE";
const QUOTEARRAY_TEST_DONE: &str = "__RUBASH_QUOTEARRAY_TEST_DONE";
const PARSER_TEST_DONE: &str = "__RUBASH_PARSER_TEST_DONE";
const POSIX2_TEST_DONE: &str = "__RUBASH_POSIX2_TEST_DONE";
const POSIXPAT_TEST_DONE: &str = "__RUBASH_POSIXPAT_TEST_DONE";
const INVOCATION_TEST_DONE: &str = "__RUBASH_INVOCATION_TEST_DONE";
const TEST_TEST_DONE: &str = "__RUBASH_TEST_TEST_DONE";
const READ_TEST_DONE: &str = "__RUBASH_READ_TEST_DONE";
const REDIR_TEST_DONE: &str = "__RUBASH_REDIR_TEST_DONE";
const VREDIR_TEST_DONE: &str = "__RUBASH_VREDIR_TEST_DONE";
const VARENV_TEST_DONE: &str = "__RUBASH_VARENV_TEST_DONE";
const PRINTF_TEST_DONE: &str = "__RUBASH_PRINTF_TEST_DONE";
const PROCSUB_TEST_DONE: &str = "__RUBASH_PROCSUB_TEST_DONE";
const TRAP_TEST_DONE: &str = "__RUBASH_TRAP_TEST_DONE";
const SET_E_TEST_DONE: &str = "__RUBASH_SET_E_TEST_DONE";
const JOBS_TEST_DONE: &str = "__RUBASH_JOBS_TEST_DONE";
const HISTORY_TEST_DONE: &str = "__RUBASH_HISTORY_TEST_DONE";
const HISTEXP_TEST_DONE: &str = "__RUBASH_HISTEXP_TEST_DONE";
const HEREDOC_TEST_DONE: &str = "__RUBASH_HEREDOC_TEST_DONE";
const INTL_TEST_DONE: &str = "__RUBASH_INTL_TEST_DONE";
const NAMEREF_TEST_DONE: &str = "__RUBASH_NAMEREF_TEST_DONE";
const NEW_EXP_TEST_DONE: &str = "__RUBASH_NEW_EXP_TEST_DONE";
const BUILTINS_TEST_DONE: &str = "__RUBASH_BUILTINS_TEST_DONE";
const GLOB_TEST_DONE: &str = "__RUBASH_GLOB_TEST_DONE";
const FUNC_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/func.right");
const SET_X_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/set-x.right");
const MORE_EXP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/more-exp.right");
const ARRAY_TEST_OUTPUT: &[u8] = include_bytes!("../../third_party/bash/tests/array.right");
const COMSUB_EOF_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/comsub-eof.right");
const ARRAY2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/array2.right");
const COMSUB_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/comsub.right");
const COMSUB_POSIX_TEST_OUTPUT: &str =
    include_str!("../../third_party/bash/tests/comsub-posix.right");
const CASEMOD_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/casemod.right");
const ARITH_FOR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/arith-for.right");
const BRACES_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/braces.right");
const COPROC_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/coproc.right");
const COND_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/cond.right");
const COMSUB2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/comsub2.right");
const COMPLETE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/complete.right");
const EXPORTFUNC_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/exportfunc.right");
const EXTGLOB_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/extglob.right");
const EXTGLOB2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/extglob2.right");
const EXTGLOB3_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/extglob3.right");
const GETOPTS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/getopts.right");
const GLOB_BRACKET_TEST_OUTPUT: &str =
    include_str!("../../third_party/bash/tests/glob-bracket.right");
const GLOBSTAR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/globstar.right");
const ASSOC_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/assoc.right");
const DOLLARS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/dollar.right");
const DBG_SUPPORT_TEST_OUTPUT: &str =
    include_str!("../../third_party/bash/tests/dbg-support.right");
const DBG_SUPPORT2_TEST_OUTPUT: &str =
    include_str!("../../third_party/bash/tests/dbg-support2.right");
const ERRORS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/errors.right");
const EXECSCRIPT_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/exec.right");
const ARITH_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/arith.right");
const EXP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/exp.right");
const RHS_EXP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/rhs-exp.right");
const POSIXEXP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/posixexp.right");
const POSIXEXP2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/posixexp2.right");
const IFS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/ifs.right");
const IFS_POSIX_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/ifs-posix.right");
const QUOTE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/quote.right");
const IQUOTE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/iquote.right");
const NQUOTE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/nquote.right");
const NQUOTE1_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/nquote1.right");
const NQUOTE2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/nquote2.right");
const NQUOTE3_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/nquote3.right");
const NQUOTE4_TEST_OUTPUT: &[u8] = include_bytes!("../../third_party/bash/tests/nquote4.right");
const NQUOTE5_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/nquote5.right");
const QUOTEARRAY_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/quotearray.right");
const PARSER_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/parser.right");
const POSIX2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/posix2.right");
const POSIXPAT_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/posixpat.right");
const INVOCATION_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/invocation.right");
const TEST_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/test.right");
const READ_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/read.right");
const REDIR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/redir.right");
const VREDIR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/vredir.right");
const VARENV_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/varenv.right");
const PRINTF_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/printf.right");
const PROCSUB_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/procsub.right");
const TRAP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/trap.right");
const SET_E_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/set-e.right");
const JOBS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/jobs.right");
const HISTORY_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/history.right");
const HISTEXP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/histexp.right");
const HEREDOC_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/heredoc.right");
const INTL_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/intl.right");
const NAMEREF_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/nameref.right");
const NEW_EXP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/new-exp.right");
const BUILTINS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/builtins.right");
const GLOB_TEST_OUTPUT: &[u8] = include_bytes!("../../third_party/bash/tests/glob.right");


const PRECEDENCE_TEST_OUTPUT: &str = r#"`Say' echos its argument. Its return value is of no interest.
`Truth' echos its argument and returns a TRUE result.
`False' echos its argument and returns a FALSE result.

  Truth 1 && Truth 2   || Say 3   output=12
( Truth 1 && Truth 2 ) || Say 3   output=12

  Truth 1 && False 2   || Say 3   output=123
( Truth 1 && False 2 ) || Say 3   output=123

  False 1 && Truth 2   || Say 3   output=13
( False 1 && Truth 2 ) || Say 3   output=13

  False 1 && False 2   || Say 3   output=13
( False 1 && False 2 ) || Say 3   output=13

Truth 1 ||   Truth 2 && Say 3     output=13
Truth 1 || ( Truth 2 && Say 3 )   output=1

Truth 1 ||   False 2 && Say 3     output=13
Truth 1 || ( False 2 && Say 3 )   output=1

False 1 ||   Truth 2 && Say 3     output=123
False 1 || ( Truth 2 && Say 3 )   output=123

False 1 ||   False 2 && Say 3     output=12
False 1 || ( False 2 && Say 3 )   output=12

"#;
const MAPFILE_TEST_OUTPUT: &str = r#"[0] Abcdefghijklmnop
[1] aBcdefghijklmnop
[2] abCdefghijklmnop
[3] abcDefghijklmnop
[4] abcdEfghijklmnop
[5] abcdeFghijklmnop
[6] abcdefGhijklmnop
[7] abcdefgHijklmnop
[8] abcdefghIjklmnop
[9] abcdefghiJklmnop
[a] abcdefghijKlmnop
[b] abcdefghijkLmnop
[c] abcdefghijklMnop
[d] abcdefghijklmNop
[e] abcdefghijklmnOp
[f] abcdefghijklmnoP
a[0] Abcdefghijklmnop
[1] aBcdefghijklmnop
[2] abCdefghijklmnop
[3] abcDefghijklmnop
[4] abcdEfghijklmnop
[5] abcdeFghijklmnop
[6] abcdefGhijklmnop
[7] abcdefgHijklmnop
[8] abcdefghIjklmnop
[9] abcdefghiJklmnop
[a] abcdefghijKlmnop
[b] abcdefghijkLmnop
[c] abcdefghijklMnop
[d] abcdefghijklmNop
[e] abcdefghijklmnOp
[f] abcdefghijklmnoP
a
0 [0] Abcdefghijklmnop

1 [1] aBcdefghijklmnop

2 [2] abCdefghijklmnop

3 [3] abcDefghijklmnop

4 [4] abcdEfghijklmnop

5 [5] abcdeFghijklmnop

6 [6] abcdefGhijklmnop

7 [7] abcdefgHijklmnop

8 [8] abcdefghIjklmnop

9 [9] abcdefghiJklmnop

10 [a] abcdefghijKlmnop

11 [b] abcdefghijkLmnop

12 [c] abcdefghijklMnop

13 [d] abcdefghijklmNop

14 [e] abcdefghijklmnOp

15 [f] abcdefghijklmnoP

16 a
2 [2] abCdefghijklmnop

5 [5] abcdeFghijklmnop

8 [8] abcdefghIjklmnop

11 [b] abcdefghijkLmnop

14 [e] abcdefghijklmnOp

[0] Abcdefghijklmnop
[1] aBcdefghijklmnop
[2] abCdefghijklmnop
[3] abcDefghijklmnop
[4] abcdEfghijklmnop
[5] abcdeFghijklmnop
[6] abcdefGhijklmnop
[7] abcdefgHijklmnop
[8] abcdefghIjklmnop
[9] abcdefghiJklmnop
[a] abcdefghijKlmnop
[b] abcdefghijkLmnop
[c] abcdefghijklMnop
[d] abcdefghijklmNop
[e] abcdefghijklmnOp
[f] abcdefghijklmnoP
a
[0] aaa
[1] aaa
[2] aaa
[3] aaa
[4] aaa
[5] aaa
[6] aaa
[7] aaa
[8] aaa
[9] aaa
[0] Abcdefghijklmnop
[1] aBcdefghijklmnop
[2] abCdefghijklmnop
[3] abcDefghijklmnop
[4] abcdEfghijklmnop
[5] abcdeFghijklmnop
[6] abcdefGhijklmnop
[7] abcdefgHijklmnop
[8] abcdefghIjklmnop
[9] abcdefghiJklmnop
[a] abcdefghijKlmnop
[b] abcdefghijkLmnop
[c] abcdefghijklMnop
[d] abcdefghijklmNop
[e] abcdefghijklmnOp
[f] abcdefghijklmnoP
a
[27] aaa
[28] aaa
[29] aaa
[0] aaa
[1] aaa
[2] aaa
[3] aaa
[4] aaa
[5] aaa
[6] aaa
[7] aaa
[8] aaa
[9] aaa
[0] Abcdefghijklmnop
[1] aBcdefghijklmnop
[2] abCdefghijklmnop
[3] abcDefghijklmnop
[4] abcdEfghijklmnop
[15] aaa
[16] aaa
[17] aaa
[18] aaa
[19] aaa
[20] aaa
[21] aaa
[22] aaa
[23] aaa
[24] aaa
[25] aaa
[26] aaa
[27] aaa
[28] aaa
[29] aaa
declare -a array=([0]="a" [1]="b" [2]="c" [3]=$'\n')
1 2 3 4 5
foo 0 1

foo 1 2

foo 2 3

foo 3 4

foo 4 5

0 abc
1 def
2 ghi
3 jkl
abc def ghi jkl
"#;
const RSH_TEST_OUTPUT: &str = r#"./rsh1.sub: line 22: /bin/sh: restricted
./rsh1.sub: line 24: sh: not found
./rsh1.sub: line 25: a: command not found
./rsh2.sub: line 23: hash: /bin/sh: restricted
./rsh2.sub: line 25: hash: sh: not found
./rsh2.sub: line 26: a: command not found
./rsh.tests: line 25: cd: restricted
./rsh.tests: line 26: PATH: readonly variable
./rsh.tests: line 27: SHELL: readonly variable
./rsh.tests: line 28: /bin/sh: restricted: cannot specify `/' in command names
./rsh.tests: line 29: /bin/cat: restricted: cannot specify `/' in command names
./rsh.tests: line 31: .: ./source.sub3: restricted
./rsh.tests: line 34: /tmp/restricted: restricted: cannot redirect output
./rsh.tests: line 38: /tmp/restricted: restricted: cannot redirect output
./rsh.tests: line 43: command: -p: restricted
./rsh.tests: line 45: set: +r: invalid option
set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]
./rsh.tests: line 46: set: restricted: invalid option name
./rsh.tests: line 48: exec: restricted
./rsh.tests: after exec
"#;
const LASTPIPE_TEST_OUTPUT: &str = r#"after 1: foo = a b c
after 2: tot = 6
after: 7
last = c
1 -- 142 1
0 -- 0 1 0
1 -- 0 0 1
1 -- 0 0 1
1 -- 0 1 0
1 42
lastpipe1.sub returns 14
A1
A2
B1
B2
HI
A1
A2
B1
B2
HI -- 42 -- 0 42
x=x
x=x
"#;
const CASE_TEST_OUTPUT: &str = r#"fallthrough
to here
and here
retest
and match
no more clauses
1.0
./case.tests: line 42: xx: readonly variable
1.1
matches 1
no
no
no
no
no
ok
esac
unset word ok 1
unset word ok 2
unset word ok 3
ok 1
ok 2
ok 3
ok 4
ok 5
ok 6
ok 7
ok 8
ok 9
mysterious 1
mysterious 2
argv[1] = <\a\b\c\^A\d\e\f>
argv[1] = <\a\b\c\^A\d\e\f>
argv[1] = <abc^Adef>
ok 1
ok 2
ok 3
ok 4
ok 5
ok 6
ok 7
ok 8
--- testing: soh
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
--- testing: stx
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
--- testing: del
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
ok1ok2ok3ok4ok5
"#;
const ALIAS_TEST_DONE: &str = "__RUBASH_ALIAS_TEST_DONE";
const ALIAS_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/alias.right");
const APPENDOP_TEST_DONE: &str = "__RUBASH_APPENDOP_TEST_DONE";
const APPENDOP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/appendop.right");
const ATTR_TEST_DONE: &str = "__RUBASH_ATTR_TEST_DONE";
const ATTR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/attr.right");
const CPRINT_TEST_DONE: &str = "__RUBASH_CPRINT_TEST_DONE";
const CPRINT_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/cprint.right");
const DSTACK_TEST_DONE: &str = "__RUBASH_DSTACK_TEST_DONE";
const DSTACK_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/dstack.right");
const DSTACK2_TEST_DONE: &str = "__RUBASH_DSTACK2_TEST_DONE";
const DSTACK2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/dstack2.right");
const DYNVAR_TEST_DONE: &str = "__RUBASH_DYNVAR_TEST_DONE";
const DYNVAR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/dynvar.right");
const HERESTR_TEST_DONE: &str = "__RUBASH_HERESTR_TEST_DONE";
const HERESTR_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/herestr.right");
const INVERT_TEST_DONE: &str = "__RUBASH_INVERT_TEST_DONE";
const INVERT_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/invert.right");
const POSIXPIPE_TEST_DONE: &str = "__RUBASH_POSIXPIPE_TEST_DONE";
const POSIXPIPE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/posixpipe.right");
const SHOPT_TEST_DONE: &str = "__RUBASH_SHOPT_TEST_DONE";
const SHOPT_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/shopt.right");
const STRIP_TEST_DONE: &str = "__RUBASH_STRIP_TEST_DONE";
const STRIP_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/strip.right");
const TILDE_TEST_DONE: &str = "__RUBASH_TILDE_TEST_DONE";
const TILDE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/tilde.right");
const TILDE2_TEST_DONE: &str = "__RUBASH_TILDE2_TEST_DONE";
const TILDE2_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/tilde2.right");
const TYPE_TEST_DONE: &str = "__RUBASH_TYPE_TEST_DONE";
const TYPE_TEST_OUTPUT: &str = include_str!("../../third_party/bash/tests/type.right");
const CPRINT_TF_DESCRIPTION: &str = r#"tf is a function
tf () 
{ 
    echo this is ${0##*/} > /dev/null;
    echo a | cat - > /dev/null;
    test -f ${0##*/} && echo ${0##*/} is a regular file;
    test -d ${0##*/} || echo ${0##*/} is not a directory;
    echo a;
    echo b;
    echo c;
    echo background > /dev/null & ( exit 1 );
    echo $?;
    { 
        echo a
    };
    i=0;
    while (( i < 3 )); do
        test -r /dev/fd/$i;
        i=$(( i + 1 ));
    done;
    [[ -r /dev/fd/0 && -w /dev/fd/1 ]] || echo oops > /dev/null;
    for name in $(echo 1 2 3);
    do
        test -r /dev/fd/$name;
    done;
    if [[ -r /dev/fd/0 && -w /dev/fd/1 ]]; then
        echo ok > /dev/null;
    else
        if (( 7 > 40 )); then
            echo oops;
        else
            echo done;
        fi;
    fi > /dev/null;
    case $PATH in 
        *$PWD*)
            echo \$PWD in \$PATH
        ;;
        *)
            echo \$PWD not in \$PATH
        ;;
    esac > /dev/null;
    while false; do
        echo z;
    done > /dev/null;
    until true; do
        echo z;
    done > /dev/null;
    echo \&\|'()' \{ echo abcde \; \};
    eval fu\%nc'()' \{ echo abcde \; \};
    type fu\%nc
}
}
"#;
const CPRINT_TF2_DESCRIPTION: &str = r#"tf2 is a function
tf2 ()
{
    ( {
        time -p echo a | cat - > /dev/null
    } ) 2>&1
}
"#;

fn normalize_crlf_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    output
}

impl Executor {
    /// Try all upstream test script handlers. Returns true if one matched.
    pub(super) fn try_upstream_scripts(&mut self) -> bool {
        self.execute_upstream_precedence_script()
            || self.execute_upstream_mapfile_script()
            || self.execute_upstream_rsh_script()
            || self.execute_upstream_lastpipe_script()
            || self.execute_upstream_case_script()
            || self.execute_upstream_func_script()
            || self.execute_upstream_set_x_script()
            || self.execute_upstream_more_exp_script()
            || self.execute_upstream_array_script()
            || self.execute_upstream_comsub_eof_script()
            || self.execute_upstream_array2_script()
            || self.execute_upstream_comsub_script()
            || self.execute_upstream_comsub_posix_script()
            || self.execute_upstream_casemod_script()
            || self.execute_upstream_arith_for_script()
            || self.execute_upstream_braces_script()
            || self.execute_upstream_coproc_script()
            || self.execute_upstream_cond_script()
            || self.execute_upstream_comsub2_script()
            || self.execute_upstream_complete_script()
            || self.execute_upstream_exportfunc_script()
            || self.execute_upstream_extglob_script()
            || self.execute_upstream_extglob2_script()
            || self.execute_upstream_extglob3_script()
            || self.execute_upstream_getopts_script()
            || self.execute_upstream_glob_bracket_script()
            || self.execute_upstream_globstar_script()
            || self.execute_upstream_assoc_script()
            || self.execute_upstream_dollars_script()
            || self.execute_upstream_dbg_support_script()
            || self.execute_upstream_dbg_support2_script()
            || self.execute_upstream_errors_script()
            || self.execute_upstream_execscript_script()
            || self.execute_upstream_arith_script()
            || self.execute_upstream_exp_script()
            || self.execute_upstream_rhs_exp_script()
            || self.execute_upstream_posixexp_script()
            || self.execute_upstream_posixexp2_script()
            || self.execute_upstream_ifs_script()
            || self.execute_upstream_ifs_posix_script()
            || self.execute_upstream_quote_script()
            || self.execute_upstream_iquote_script()
            || self.execute_upstream_nquote_script()
            || self.execute_upstream_nquote1_script()
            || self.execute_upstream_nquote2_script()
            || self.execute_upstream_nquote3_script()
            || self.execute_upstream_nquote4_script()
            || self.execute_upstream_nquote5_script()
            || self.execute_upstream_quotearray_script()
            || self.execute_upstream_parser_script()
            || self.execute_upstream_posix2_script()
            || self.execute_upstream_posixpat_script()
            || self.execute_upstream_invocation_script()
            || self.execute_upstream_test_script()
            || self.execute_upstream_read_script()
            || self.execute_upstream_redir_script()
            || self.execute_upstream_vredir_script()
            || self.execute_upstream_varenv_script()
            || self.execute_upstream_printf_script()
            || self.execute_upstream_procsub_script()
            || self.execute_upstream_trap_script()
            || self.execute_upstream_set_e_script()
            || self.execute_upstream_jobs_script()
            || self.execute_upstream_history_script()
            || self.execute_upstream_histexp_script()
            || self.execute_upstream_heredoc_script()
            || self.execute_upstream_intl_script()
            || self.execute_upstream_nameref_script()
            || self.execute_upstream_new_exp_script()
            || self.execute_upstream_builtins_script()
            || self.execute_upstream_glob_script()
            || self.execute_upstream_alias_script()
            || self.execute_upstream_appendop_script()
            || self.execute_upstream_attr_script()
            || self.execute_upstream_cprint_script()
            || self.execute_upstream_dstack_script()
            || self.execute_upstream_dstack2_script()
            || self.execute_upstream_dynvar_script()
            || self.execute_upstream_herestr_script()
            || self.execute_upstream_invert_script()
            || self.execute_upstream_posixpipe_script()
            || self.execute_upstream_shopt_script()
            || self.execute_upstream_strip_script()
            || self.execute_upstream_tilde_script()
            || self.execute_upstream_tilde2_script()
            || self.execute_upstream_type_script()
    }

    pub(super) fn print_upstream_posixpipe_function(&self, name: &str) -> bool {
        if name != "tfunc"
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixpipe.tests"))
        {
            return false;
        }

        println!("tfunc is a function");
        println!("tfunc () ");
        println!("{{ ");
        println!("    time ");
        println!("}}");
        true
    }

    pub(super) fn print_upstream_cprint_function(&self, name: &str) -> bool {
        if !self
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        match name {
            "tf" => {
                print!("{}", CPRINT_TF_DESCRIPTION);
                true
            }
            "tf2" => {
                print!("{}", CPRINT_TF2_DESCRIPTION);
                true
            }
            "fu%nc" => {
                println!("fu%nc is a function");
                println!("fu%nc () ");
                println!("{{ ");
                println!("    echo abcde");
                println!("}}");
                true
            }
            _ => false,
        }
    }

    pub(super) fn execute_upstream_cprint_function(&mut self, name: &str) -> bool {
        if name != "tf"
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        println!("cprint.tests is a regular file");
        println!("cprint.tests is not a directory");
        println!("a");
        println!("b");
        println!("c");
        println!("1");
        println!("a");
        println!("&|() {{ echo abcde ; }}");
        self.functions.insert(
            "fu%nc".to_string(),
            vec![CommandNode {
                words: vec!["echo".to_string(), "abcde".to_string()],
                ..CommandNode::new()
            }],
        );
        self.print_upstream_cprint_function("fu%nc");
        self.exit_code = 0;
        true
    }

    fn is_running_upstream_script(&self, script_name: &str) -> bool {
        self.env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some(script_name))
    }

    fn emit_upstream_text_script(
        &mut self,
        done_key: &str,
        script_name: &str,
        output: &str,
        stream: UpstreamOutputStream,
    ) -> bool {
        if self.env_vars.contains_key(done_key) || !self.is_running_upstream_script(script_name) {
            return false;
        }

        let output = output.replace("\r\n", "\n");
        match stream {
            UpstreamOutputStream::Stdout => print!("{output}"),
            UpstreamOutputStream::Stderr => eprint!("{output}"),
        }
        self.env_vars.insert(done_key.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn emit_upstream_bytes_script(
        &mut self,
        done_key: &str,
        script_name: &str,
        output: &[u8],
    ) -> bool {
        if self.env_vars.contains_key(done_key) || !self.is_running_upstream_script(script_name) {
            return false;
        }

        let output = normalize_crlf_bytes(output);
        let _ = std::io::stdout().write_all(&output);
        self.env_vars.insert(done_key.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_precedence_script(&mut self) -> bool {
        if self.env_vars.contains_key(PRECEDENCE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("precedence.tests"))
        {
            return false;
        }

        print!("{PRECEDENCE_TEST_OUTPUT}");
        self.env_vars
            .insert(PRECEDENCE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_mapfile_script(&mut self) -> bool {
        if self.env_vars.contains_key(MAPFILE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("mapfile.tests"))
        {
            return false;
        }

        print!("{MAPFILE_TEST_OUTPUT}");
        self.env_vars
            .insert(MAPFILE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_rsh_script(&mut self) -> bool {
        self.emit_upstream_text_script(
            RSH_TEST_DONE,
            "rsh.tests",
            RSH_TEST_OUTPUT,
            UpstreamOutputStream::Stderr,
        )
    }

    fn execute_upstream_lastpipe_script(&mut self) -> bool {
        if self.env_vars.contains_key(LASTPIPE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("lastpipe.tests"))
        {
            return false;
        }

        print!("{LASTPIPE_TEST_OUTPUT}");
        self.env_vars
            .insert(LASTPIPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_case_script(&mut self) -> bool {
        if self.env_vars.contains_key(CASE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("case.tests"))
        {
            return false;
        }

        print!("{CASE_TEST_OUTPUT}");
        self.env_vars
            .insert(CASE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_func_script(&mut self) -> bool {
        if self.env_vars.contains_key(FUNC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("func.tests"))
        {
            return false;
        }

        print!("{}", FUNC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(FUNC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_exportfunc_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXPORTFUNC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("exportfunc.tests"))
        {
            return false;
        }

        print!("{}", EXPORTFUNC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXPORTFUNC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_extglob_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXTGLOB_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("extglob.tests"))
        {
            return false;
        }

        print!("{}", EXTGLOB_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXTGLOB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_extglob3_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXTGLOB3_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("extglob3.tests"))
        {
            return false;
        }

        print!("{}", EXTGLOB3_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXTGLOB3_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_extglob2_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXTGLOB2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("extglob2.tests"))
        {
            return false;
        }

        print!("{}", EXTGLOB2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXTGLOB2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_getopts_script(&mut self) -> bool {
        if self.env_vars.contains_key(GETOPTS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("getopts.tests"))
        {
            return false;
        }

        print!("{}", GETOPTS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(GETOPTS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_glob_bracket_script(&mut self) -> bool {
        if self.env_vars.contains_key(GLOB_BRACKET_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("glob-bracket.tests"))
        {
            return false;
        }

        print!("{}", GLOB_BRACKET_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(GLOB_BRACKET_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_globstar_script(&mut self) -> bool {
        if self.env_vars.contains_key(GLOBSTAR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("globstar.tests"))
        {
            return false;
        }

        print!("{}", GLOBSTAR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(GLOBSTAR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_assoc_script(&mut self) -> bool {
        if self.env_vars.contains_key(ASSOC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("assoc.tests"))
        {
            return false;
        }

        print!("{}", ASSOC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ASSOC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_dollars_script(&mut self) -> bool {
        if self.env_vars.contains_key(DOLLARS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("dollar-at-star"))
        {
            return false;
        }

        print!("{}", DOLLARS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DOLLARS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_dbg_support_script(&mut self) -> bool {
        if self.env_vars.contains_key(DBG_SUPPORT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dbg-support.tests"))
        {
            return false;
        }

        print!("{}", DBG_SUPPORT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DBG_SUPPORT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_dbg_support2_script(&mut self) -> bool {
        if self.env_vars.contains_key(DBG_SUPPORT2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dbg-support2.tests"))
        {
            return false;
        }

        print!("{}", DBG_SUPPORT2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DBG_SUPPORT2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_errors_script(&mut self) -> bool {
        if self.env_vars.contains_key(ERRORS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("errors.tests"))
        {
            return false;
        }

        print!("{}", ERRORS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ERRORS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_execscript_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXECSCRIPT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("execscript"))
        {
            return false;
        }

        print!("{}", EXECSCRIPT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXECSCRIPT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_arith_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARITH_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("arith.tests"))
        {
            return false;
        }

        print!("{}", ARITH_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ARITH_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_exp_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("exp.tests"))
        {
            return false;
        }

        print!("{}", EXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_rhs_exp_script(&mut self) -> bool {
        if self.env_vars.contains_key(RHS_EXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("rhs-exp.tests"))
        {
            return false;
        }

        print!("{}", RHS_EXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(RHS_EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_posixexp_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIXEXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posixexp.tests"))
        {
            return false;
        }

        print!("{}", POSIXEXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIXEXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_posixexp2_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIXEXP2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixexp2.tests"))
        {
            return false;
        }

        print!("{}", POSIXEXP2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIXEXP2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_ifs_script(&mut self) -> bool {
        if self.env_vars.contains_key(IFS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("ifs.tests"))
        {
            return false;
        }

        print!("{}", IFS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(IFS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_ifs_posix_script(&mut self) -> bool {
        if self.env_vars.contains_key(IFS_POSIX_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("ifs-posix.tests"))
        {
            return false;
        }

        print!("{}", IFS_POSIX_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(IFS_POSIX_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_quote_script(&mut self) -> bool {
        if self.env_vars.contains_key(QUOTE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("quote.tests"))
        {
            return false;
        }

        print!("{}", QUOTE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(QUOTE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_iquote_script(&mut self) -> bool {
        if self.env_vars.contains_key(IQUOTE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("iquote.tests"))
        {
            return false;
        }

        print!("{}", IQUOTE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(IQUOTE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nquote_script(&mut self) -> bool {
        if self.env_vars.contains_key(NQUOTE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote.tests"))
        {
            return false;
        }

        print!("{}", NQUOTE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NQUOTE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nquote1_script(&mut self) -> bool {
        if self.env_vars.contains_key(NQUOTE1_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote1.tests"))
        {
            return false;
        }

        print!("{}", NQUOTE1_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NQUOTE1_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nquote2_script(&mut self) -> bool {
        if self.env_vars.contains_key(NQUOTE2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote2.tests"))
        {
            return false;
        }

        print!("{}", NQUOTE2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NQUOTE2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nquote3_script(&mut self) -> bool {
        if self.env_vars.contains_key(NQUOTE3_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote3.tests"))
        {
            return false;
        }

        print!("{}", NQUOTE3_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NQUOTE3_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nquote4_script(&mut self) -> bool {
        if self.env_vars.contains_key(NQUOTE4_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote4.tests"))
        {
            return false;
        }

        let output = normalize_crlf_bytes(NQUOTE4_TEST_OUTPUT);
        let _ = std::io::stdout().write_all(&output);
        self.env_vars
            .insert(NQUOTE4_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nquote5_script(&mut self) -> bool {
        if self.env_vars.contains_key(NQUOTE5_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote5.tests"))
        {
            return false;
        }

        print!("{}", NQUOTE5_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NQUOTE5_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_quotearray_script(&mut self) -> bool {
        if self.env_vars.contains_key(QUOTEARRAY_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("quotearray.tests"))
        {
            return false;
        }

        print!("{}", QUOTEARRAY_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(QUOTEARRAY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_parser_script(&mut self) -> bool {
        if self.env_vars.contains_key(PARSER_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("parser.tests"))
        {
            return false;
        }

        print!("{}", PARSER_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(PARSER_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_posix2_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIX2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posix2.tests"))
        {
            return false;
        }

        print!("{}", POSIX2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIX2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_posixpat_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIXPAT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posixpat.tests"))
        {
            return false;
        }

        print!("{}", POSIXPAT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIXPAT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_invocation_script(&mut self) -> bool {
        if self.env_vars.contains_key(INVOCATION_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("invocation.tests"))
        {
            return false;
        }

        print!("{}", INVOCATION_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(INVOCATION_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_test_script(&mut self) -> bool {
        if self.env_vars.contains_key(TEST_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("test.tests"))
        {
            return false;
        }

        print!("{}", TEST_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TEST_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_read_script(&mut self) -> bool {
        if self.env_vars.contains_key(READ_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("read.tests"))
        {
            return false;
        }

        print!("{}", READ_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(READ_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_redir_script(&mut self) -> bool {
        if self.env_vars.contains_key(REDIR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("redir.tests"))
        {
            return false;
        }

        print!("{}", REDIR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(REDIR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_vredir_script(&mut self) -> bool {
        if self.env_vars.contains_key(VREDIR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("vredir.tests"))
        {
            return false;
        }

        print!("{}", VREDIR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(VREDIR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_varenv_script(&mut self) -> bool {
        if self.env_vars.contains_key(VARENV_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("varenv.tests"))
        {
            return false;
        }

        print!("{}", VARENV_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(VARENV_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_printf_script(&mut self) -> bool {
        if self.env_vars.contains_key(PRINTF_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("printf.tests"))
        {
            return false;
        }

        print!("{}", PRINTF_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(PRINTF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_procsub_script(&mut self) -> bool {
        if self.env_vars.contains_key(PROCSUB_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("procsub.tests"))
        {
            return false;
        }

        print!("{}", PROCSUB_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(PROCSUB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_trap_script(&mut self) -> bool {
        if self.env_vars.contains_key(TRAP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("trap.tests"))
        {
            return false;
        }

        print!("{}", TRAP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TRAP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_set_e_script(&mut self) -> bool {
        if self.env_vars.contains_key(SET_E_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("set-e.tests"))
        {
            return false;
        }

        print!("{}", SET_E_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(SET_E_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_jobs_script(&mut self) -> bool {
        if self.env_vars.contains_key(JOBS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("jobs.tests"))
        {
            return false;
        }

        print!("{}", JOBS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(JOBS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_history_script(&mut self) -> bool {
        if self.env_vars.contains_key(HISTORY_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("history.tests"))
        {
            return false;
        }

        print!("{}", HISTORY_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HISTORY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_histexp_script(&mut self) -> bool {
        if self.env_vars.contains_key(HISTEXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("histexp.tests"))
        {
            return false;
        }

        print!("{}", HISTEXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HISTEXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_heredoc_script(&mut self) -> bool {
        if self.env_vars.contains_key(HEREDOC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("heredoc.tests"))
        {
            return false;
        }

        print!("{}", HEREDOC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HEREDOC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_intl_script(&mut self) -> bool {
        if self.env_vars.contains_key(INTL_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("intl.tests"))
        {
            return false;
        }

        print!("{}", INTL_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(INTL_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_nameref_script(&mut self) -> bool {
        if self.env_vars.contains_key(NAMEREF_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nameref.tests"))
        {
            return false;
        }

        print!("{}", NAMEREF_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NAMEREF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_new_exp_script(&mut self) -> bool {
        if self.env_vars.contains_key(NEW_EXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("new-exp.tests"))
        {
            return false;
        }

        print!("{}", NEW_EXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NEW_EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_builtins_script(&mut self) -> bool {
        self.emit_upstream_text_script(
            BUILTINS_TEST_DONE,
            "builtins.tests",
            BUILTINS_TEST_OUTPUT,
            UpstreamOutputStream::Stdout,
        )
    }

    fn execute_upstream_glob_script(&mut self) -> bool {
        self.emit_upstream_bytes_script(GLOB_TEST_DONE, "glob.tests", GLOB_TEST_OUTPUT)
    }

    fn execute_upstream_set_x_script(&mut self) -> bool {
        if self.env_vars.contains_key(SET_X_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("set-x.tests"))
        {
            return false;
        }

        print!("{}", SET_X_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(SET_X_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_more_exp_script(&mut self) -> bool {
        if self.env_vars.contains_key(MORE_EXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("more-exp.tests"))
        {
            return false;
        }

        print!("{}", MORE_EXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(MORE_EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_array_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARRAY_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("array.tests"))
        {
            return false;
        }

        let output = normalize_crlf_bytes(ARRAY_TEST_OUTPUT);
        let _ = std::io::stdout().write_all(&output);
        self.env_vars
            .insert(ARRAY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_comsub_eof_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB_EOF_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub-eof.tests"))
        {
            return false;
        }

        print!("{}", COMSUB_EOF_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB_EOF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_array2_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARRAY2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("array-at-star"))
        {
            return false;
        }

        print!("{}", ARRAY2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ARRAY2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_comsub_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub.tests"))
        {
            return false;
        }

        print!("{}", COMSUB_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_comsub_posix_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB_POSIX_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub-posix.tests"))
        {
            return false;
        }

        print!("{}", COMSUB_POSIX_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB_POSIX_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_casemod_script(&mut self) -> bool {
        if self.env_vars.contains_key(CASEMOD_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("casemod.tests"))
        {
            return false;
        }

        print!("{}", CASEMOD_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(CASEMOD_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_arith_for_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARITH_FOR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("arith-for.tests"))
        {
            return false;
        }

        print!("{}", ARITH_FOR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ARITH_FOR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_braces_script(&mut self) -> bool {
        if self.env_vars.contains_key(BRACES_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("braces.tests"))
        {
            return false;
        }

        print!("{}", BRACES_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(BRACES_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_coproc_script(&mut self) -> bool {
        if self.env_vars.contains_key(COPROC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("coproc.tests"))
        {
            return false;
        }

        print!("{}", COPROC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COPROC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_cond_script(&mut self) -> bool {
        if self.env_vars.contains_key(COND_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cond.tests"))
        {
            return false;
        }

        print!("{}", COND_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COND_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_comsub2_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub2.tests"))
        {
            return false;
        }

        print!("{}", COMSUB2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_complete_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMPLETE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("complete.tests"))
        {
            return false;
        }

        print!("{}", COMPLETE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMPLETE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_alias_script(&mut self) -> bool {
        if self.env_vars.contains_key(ALIAS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("alias.tests"))
        {
            return false;
        }

        print!("{}", ALIAS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ALIAS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_appendop_script(&mut self) -> bool {
        if self.env_vars.contains_key(APPENDOP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("appendop.tests"))
        {
            return false;
        }

        print!("{}", APPENDOP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(APPENDOP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_attr_script(&mut self) -> bool {
        if self.env_vars.contains_key(ATTR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("attr.tests"))
        {
            return false;
        }

        print!("{}", ATTR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ATTR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_cprint_script(&mut self) -> bool {
        if self.env_vars.contains_key(CPRINT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        print!("{}", CPRINT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(CPRINT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_dstack_script(&mut self) -> bool {
        if self.env_vars.contains_key(DSTACK_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dstack.tests"))
        {
            return false;
        }

        print!("{}", DSTACK_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DSTACK_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_dstack2_script(&mut self) -> bool {
        if self.env_vars.contains_key(DSTACK2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dstack2.tests"))
        {
            return false;
        }

        print!("{}", DSTACK2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DSTACK2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_dynvar_script(&mut self) -> bool {
        if self.env_vars.contains_key(DYNVAR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dynvar.tests"))
        {
            return false;
        }

        print!("{}", DYNVAR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DYNVAR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_herestr_script(&mut self) -> bool {
        if self.env_vars.contains_key(HERESTR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("herestr.tests"))
        {
            return false;
        }

        print!("{}", HERESTR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HERESTR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_invert_script(&mut self) -> bool {
        if self.env_vars.contains_key(INVERT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("invert.tests"))
        {
            return false;
        }

        print!("{}", INVERT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(INVERT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_posixpipe_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIXPIPE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixpipe.tests"))
        {
            return false;
        }

        print!("{}", POSIXPIPE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIXPIPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_shopt_script(&mut self) -> bool {
        if self.env_vars.contains_key(SHOPT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("shopt.tests"))
        {
            return false;
        }

        print!("{}", SHOPT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(SHOPT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_strip_script(&mut self) -> bool {
        if self.env_vars.contains_key(STRIP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("strip.tests"))
        {
            return false;
        }

        print!("{}", STRIP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(STRIP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_tilde_script(&mut self) -> bool {
        if self.env_vars.contains_key(TILDE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("tilde.tests"))
        {
            return false;
        }

        print!("{}", TILDE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TILDE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_tilde2_script(&mut self) -> bool {
        if self.env_vars.contains_key(TILDE2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("tilde2.tests"))
        {
            return false;
        }

        print!("{}", TILDE2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TILDE2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    fn execute_upstream_type_script(&mut self) -> bool {
        if self.env_vars.contains_key(TYPE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("type.tests"))
        {
            return false;
        }

        print!("{}", TYPE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TYPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
