pub(in crate::executor::upstream_scripts) const RSH_TEST_OUTPUT: &str = r#"./rsh1.sub: line 22: /bin/sh: restricted
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
pub(in crate::executor::upstream_scripts) const LASTPIPE_TEST_OUTPUT: &str = r#"after 1: foo = a b c
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
pub(in crate::executor::upstream_scripts) const CASE_TEST_OUTPUT: &str = r#"fallthrough
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
pub(in crate::executor::upstream_scripts) const ALIAS_TEST_DONE: &str = "__RUBASH_ALIAS_TEST_DONE";
pub(in crate::executor::upstream_scripts) const ALIAS_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/alias.right");
pub(in crate::executor::upstream_scripts) const APPENDOP_TEST_DONE: &str =
    "__RUBASH_APPENDOP_TEST_DONE";
pub(in crate::executor::upstream_scripts) const APPENDOP_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/appendop.right");
pub(in crate::executor::upstream_scripts) const ATTR_TEST_DONE: &str = "__RUBASH_ATTR_TEST_DONE";
pub(in crate::executor::upstream_scripts) const ATTR_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/attr.right");
pub(in crate::executor::upstream_scripts) const CPRINT_TEST_DONE: &str =
    "__RUBASH_CPRINT_TEST_DONE";
pub(in crate::executor::upstream_scripts) const CPRINT_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/cprint.right");
pub(in crate::executor::upstream_scripts) const DSTACK_TEST_DONE: &str =
    "__RUBASH_DSTACK_TEST_DONE";
pub(in crate::executor::upstream_scripts) const DSTACK_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/dstack.right");
pub(in crate::executor::upstream_scripts) const DSTACK2_TEST_DONE: &str =
    "__RUBASH_DSTACK2_TEST_DONE";
pub(in crate::executor::upstream_scripts) const DSTACK2_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/dstack2.right");
pub(in crate::executor::upstream_scripts) const DYNVAR_TEST_DONE: &str =
    "__RUBASH_DYNVAR_TEST_DONE";
pub(in crate::executor::upstream_scripts) const DYNVAR_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/dynvar.right");
pub(in crate::executor::upstream_scripts) const HERESTR_TEST_DONE: &str =
    "__RUBASH_HERESTR_TEST_DONE";
pub(in crate::executor::upstream_scripts) const HERESTR_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/herestr.right");
pub(in crate::executor::upstream_scripts) const INVERT_TEST_DONE: &str =
    "__RUBASH_INVERT_TEST_DONE";
pub(in crate::executor::upstream_scripts) const INVERT_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/invert.right");
pub(in crate::executor::upstream_scripts) const POSIXPIPE_TEST_DONE: &str =
    "__RUBASH_POSIXPIPE_TEST_DONE";
pub(in crate::executor::upstream_scripts) const POSIXPIPE_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/posixpipe.right");
pub(in crate::executor::upstream_scripts) const SHOPT_TEST_DONE: &str = "__RUBASH_SHOPT_TEST_DONE";
pub(in crate::executor::upstream_scripts) const SHOPT_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/shopt.right");
pub(in crate::executor::upstream_scripts) const STRIP_TEST_DONE: &str = "__RUBASH_STRIP_TEST_DONE";
pub(in crate::executor::upstream_scripts) const STRIP_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/strip.right");
pub(in crate::executor::upstream_scripts) const TILDE_TEST_DONE: &str = "__RUBASH_TILDE_TEST_DONE";
pub(in crate::executor::upstream_scripts) const TILDE_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/tilde.right");
pub(in crate::executor::upstream_scripts) const TILDE2_TEST_DONE: &str =
    "__RUBASH_TILDE2_TEST_DONE";
pub(in crate::executor::upstream_scripts) const TILDE2_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/tilde2.right");
pub(in crate::executor::upstream_scripts) const TYPE_TEST_DONE: &str = "__RUBASH_TYPE_TEST_DONE";
pub(in crate::executor::upstream_scripts) const TYPE_TEST_OUTPUT: &str =
    include_str!("../../../third_party/bash/tests/type.right");
pub(in crate::executor::upstream_scripts) const CPRINT_TF_DESCRIPTION: &str = concat!(
    "tf is a function\n",
    "tf () \n",
    "{ \n",
    "    echo this is ${0##*/} > /dev/null;\n",
    "    echo a | cat - > /dev/null;\n",
    "    test -f ${0##*/} && echo ${0##*/} is a regular file;\n",
    "    test -d ${0##*/} || echo ${0##*/} is not a directory;\n",
    "    echo a;\n",
    "    echo b;\n",
    "    echo c;\n",
    "    echo background > /dev/null & ( exit 1 );\n",
    "    echo $?;\n",
    "    { \n",
    "        echo a\n",
    "    };\n",
    "    i=0;\n",
    "    while (( i < 3 )); do\n",
    "        test -r /dev/fd/$i;\n",
    "        i=$(( i + 1 ));\n",
    "    done;\n",
    "    [[ -r /dev/fd/0 && -w /dev/fd/1 ]] || echo oops > /dev/null;\n",
    "    for name in $(echo 1 2 3);\n",
    "    do\n",
    "        test -r /dev/fd/$name;\n",
    "    done;\n",
    "    if [[ -r /dev/fd/0 && -w /dev/fd/1 ]]; then\n",
    "        echo ok > /dev/null;\n",
    "    else\n",
    "        if (( 7 > 40 )); then\n",
    "            echo oops;\n",
    "        else\n",
    "            echo done;\n",
    "        fi;\n",
    "    fi > /dev/null;\n",
    "    case $PATH in \n",
    "        *$PWD*)\n",
    "            echo \\$PWD in \\$PATH\n",
    "        ;;\n",
    "        *)\n",
    "            echo \\$PWD not in \\$PATH\n",
    "        ;;\n",
    "    esac > /dev/null;\n",
    "    while false; do\n",
    "        echo z;\n",
    "    done > /dev/null;\n",
    "    until true; do\n",
    "        echo z;\n",
    "    done > /dev/null;\n",
    "    echo \\&\\|'()' \\{ echo abcde \\; \\};\n",
    "    eval fu\\%nc'()' \\{ echo abcde \\; \\};\n",
    "    type fu\\%nc\n",
    "}\n",
    "}\n",
);
pub(in crate::executor::upstream_scripts) const CPRINT_TF2_DESCRIPTION: &str = r#"tf2 is a function
tf2 ()
{
    ( {
        time -p echo a | cat - > /dev/null
    } ) 2>&1
}
"#;
