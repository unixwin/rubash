//! Oils-spec vendor: spec/array-assoc.test.sh.
//!
//! Vendored from the Oils shell specification suite (Apache License 2.0):
//!   https://github.com/oils-for-unix/oils/blob/master/spec/array-assoc.test.sh
//!   https://github.com/oils-for-unix/oils/blob/master/spec/append.test.sh
//!
//! Transcription rules (see task): the `#### case` / `## STDOUT` / `## OK`
//! blocks were rewritten as `Command::new(env!("CARGO_BIN_EXE_rubash"))`
//! assertions in this file's style. `argv.py` prints became
//! `printf '<%s>\n'` lines and `$TMP` fixtures became sandbox-local files.
//! Expected values were measured with the local GNU bash 5.3.20
//! (`bash -c`, file-free sandbox, `LC_ALL=C`) — never copied from an oils
//! `## OK` line, which may record oils behavior instead of bash behavior.
//! Only cases where rubash already matches bash byte-for-byte are kept;
//! the divergences found while probing are listed in the task report.

use std::fs;
use std::process::Command;

fn sandbox(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-oils-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos(),
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn run(script: &str) -> (i32, String, String) {
    let dir = sandbox("case");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        // The recorded GNU baseline was measured with `bash -c`, where
        // the word after the command string binds $0 to "bash".
        .arg("bash")
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("HOME", &dir)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env("TMPDIR", &dir)
        .output()
        .expect("spawn rubash");
    let _ = fs::remove_dir_all(&dir);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn append_array() {
    // Oils `####` case: append/array
    let (status, stdout, stderr) = run(r#"a=(x y )
a+=(t 'u v')
printf '<%s>\n' "${a[@]}""#);
    assert_eq!(status, 0, "append/array status");
    assert_eq!(
        stdout,
        r"<x>
<y>
<t>
<u v>
",
        "append/array stdout"
    );
    assert_eq!(stderr, r"", "append/array stderr");
}

#[test]
fn append_array_to_undef() {
    // Oils `####` case: append/array-to-undef
    let (status, stdout, stderr) = run(r#"y+=(c d)
printf '<%s>\n' "${y[@]}""#);
    assert_eq!(status, 0, "append/array-to-undef status");
    assert_eq!(
        stdout,
        r"<c>
<d>
",
        "append/array-to-undef stdout"
    );
    assert_eq!(stderr, r"", "append/array-to-undef stderr");
}

#[test]
fn append_elem() {
    // Oils `####` case: append/elem
    let (status, stdout, stderr) = run(r#"a=(x y )
a[1]+=z
printf '<%s>\n' "${a[@]}""#);
    assert_eq!(status, 0, "append/elem status");
    assert_eq!(
        stdout,
        r"<x>
<yz>
",
        "append/elem stdout"
    );
    assert_eq!(stderr, r"", "append/elem stderr");
}

#[test]
fn append_env_prefix() {
    // Oils `####` case: append/env-prefix
    let (status, stdout, stderr) = run(r"A=a
A+=a printenv A");
    assert_eq!(status, 0, "append/env-prefix status");
    assert_eq!(
        stdout,
        r"aa
",
        "append/env-prefix stdout"
    );
    assert_eq!(stderr, r"", "append/env-prefix stderr");
}

#[test]
fn append_export_readonly_nounset() {
    // Oils `####` case: append/export-readonly-nounset
    let (status, stdout, stderr) = run(r"export e+=foo
echo e=$e
readonly r+=bar
echo r=$r
set -u
export e+=foo
echo e=$e");
    assert_eq!(status, 0, "append/export-readonly-nounset status");
    assert_eq!(
        stdout,
        r"e=foo
r=bar
e=foofoo
",
        "append/export-readonly-nounset stdout"
    );
    assert_eq!(stderr, r"", "append/export-readonly-nounset stderr");
}

#[test]
fn append_list_to_elem_err() {
    // Oils `####` case: append/list-to-elem-err
    let (status, stdout, stderr) = run(r#"a=(1 '2 3')
a[-1]+=(4 5)
printf '<%s>\n' "${a[@]}""#);
    assert_eq!(status, 0, "append/list-to-elem-err status");
    assert_eq!(
        stdout,
        r"<1>
<2 3>
",
        "append/list-to-elem-err stdout"
    );
    assert_eq!(
        stderr,
        r"bash: line 2: a[-1]: cannot assign list to array member
",
        "append/list-to-elem-err stderr"
    );
}

#[test]
fn append_list_to_string() {
    // Oils `####` case: append/list-to-string
    let (status, stdout, stderr) = run(r#"s='abc'
s+=(d e f)
printf '<%s>\n' "${s[@]}""#);
    assert_eq!(status, 0, "append/list-to-string status");
    assert_eq!(
        stdout,
        r"<abc>
<d>
<e>
<f>
",
        "append/list-to-string stdout"
    );
    assert_eq!(stderr, r"", "append/list-to-string stderr");
}

#[test]
fn append_local_nounset() {
    // Oils `####` case: append/local-nounset
    let (status, stdout, stderr) = run(r"f() {
  local s+=foo
  echo s=$s
  set -u
  local s+=foo
  echo s=$s
}
f");
    assert_eq!(status, 0, "append/local-nounset status");
    assert_eq!(
        stdout,
        r"s=foo
s=foofoo
",
        "append/local-nounset stdout"
    );
    assert_eq!(stderr, r"", "append/local-nounset stderr");
}

#[test]
fn append_negative_elem() {
    // Oils `####` case: append/negative-elem
    let (status, stdout, stderr) = run(r#"a=(1 '2 3')
a[-1]+=' 4'
printf '<%s>\n' "${a[@]}""#);
    assert_eq!(status, 0, "append/negative-elem status");
    assert_eq!(
        stdout,
        r"<1>
<2 3 4>
",
        "append/negative-elem stdout"
    );
    assert_eq!(stderr, r"", "append/negative-elem stderr");
}

#[test]
fn append_string() {
    // Oils `####` case: append/string
    let (status, stdout, stderr) = run(r"s='abc'
s+=d
echo $s");
    assert_eq!(status, 0, "append/string status");
    assert_eq!(
        stdout,
        r"abcd
",
        "append/string stdout"
    );
    assert_eq!(stderr, r"", "append/string stderr");
}

#[test]
fn append_string_to_array_elem0() {
    // Oils `####` case: append/string-to-array-elem0
    let (status, stdout, stderr) = run(r#"a=(x y )
a+=z
printf '<%s>\n' "${a[@]}""#);
    assert_eq!(status, 0, "append/string-to-array-elem0 status");
    assert_eq!(
        stdout,
        r"<xz>
<y>
",
        "append/string-to-array-elem0 stdout"
    );
    assert_eq!(stderr, r"", "append/string-to-array-elem0 stderr");
}

#[test]
fn append_type_mismatch() {
    // Oils `####` case: append/type-mismatch
    let (status, stdout, stderr) = run(r"s=
a=()
declare -A d=([lemon]=yellow)
s+=(1)
s+=([melon]=green)
a+=lime
a+=([1]=banana)
d+=orange
d+=(0)
true");
    assert_eq!(status, 0, "append/type-mismatch status");
    assert_eq!(stdout, r"", "append/type-mismatch stdout");
    assert_eq!(stderr, r"", "append/type-mismatch stderr");
}

#[test]
fn append_typeset_dynamic_nounset() {
    // Oils `####` case: append/typeset-dynamic-nounset
    let (status, stdout, stderr) = run(r"dyn=x
typeset s${dyn}+=foo
echo sx=$sx
set -u
typeset t${dyn}+=foo
echo tx=$tx
typeset t${dyn}+=foo
echo tx=$tx");
    assert_eq!(status, 0, "append/typeset-dynamic-nounset status");
    assert_eq!(
        stdout,
        r"sx=foo
tx=foo
tx=foofoo
",
        "append/typeset-dynamic-nounset stdout"
    );
    assert_eq!(stderr, r"", "append/typeset-dynamic-nounset stderr");
}

#[test]
fn append_typeset_string_nounset() {
    // Oils `####` case: append/typeset-string-nounset
    let (status, stdout, stderr) = run(r"typeset s+=foo
echo s=$s
set -u
typeset t+=foo
echo t=$t
typeset t+=foo
echo t=$t");
    assert_eq!(status, 0, "append/typeset-string-nounset status");
    assert_eq!(
        stdout,
        r"s=foo
t=foo
t=foofoo
",
        "append/typeset-string-nounset stdout"
    );
    assert_eq!(stderr, r"", "append/typeset-string-nounset stderr");
}

#[test]
fn append_undef_with_nounset() {
    // Oils `####` case: append/undef-with-nounset
    let (status, stdout, stderr) = run(r"s+=foo
echo s=$s
set -u
t+=foo
echo t=$t
t+=foo
echo t=$t");
    assert_eq!(status, 0, "append/undef-with-nounset status");
    assert_eq!(
        stdout,
        r"s=foo
t=foo
t=foofoo
",
        "append/undef-with-nounset stdout"
    );
    assert_eq!(stderr, r"", "append/undef-with-nounset stderr");
}

#[test]
fn append_value_semantics() {
    // Oils `####` case: append/value-semantics
    let (status, stdout, stderr) = run(r"s1='abc'
s2=$s1
s1+='d'
echo $s1 $s2");
    assert_eq!(status, 0, "append/value-semantics status");
    assert_eq!(
        stdout,
        r"abcd abc
",
        "append/value-semantics stdout"
    );
    assert_eq!(stderr, r"", "append/value-semantics stderr");
}

#[test]
fn assoc_append_compound() {
    // Oils `####` case: assoc/append-compound
    let (status, stdout, stderr) = run(r#"declare -A a=([apple]=red [orange]=orange)
a+=([lemon]=yellow [banana]=yellow)
echo "apple is ${a['apple']}"
echo "orange is ${a['orange']}"
echo "lemon is ${a['lemon']}"
echo "banana is ${a['banana']}""#);
    assert_eq!(status, 0, "assoc/append-compound status");
    assert_eq!(
        stdout,
        r"apple is red
orange is orange
lemon is yellow
banana is yellow
",
        "assoc/append-compound stdout"
    );
    assert_eq!(stderr, r"", "assoc/append-compound stderr");
}

#[test]
fn assoc_arith_keys_numeric() {
    // Oils `####` case: assoc/arith-keys-numeric
    let (status, stdout, stderr) = run(r"declare -A assoc
assoc[0]=42
(( var = ${assoc[0]} ))
echo $var
(( var = assoc[0] ))
echo $var");
    assert_eq!(status, 0, "assoc/arith-keys-numeric status");
    assert_eq!(
        stdout,
        r"42
42
",
        "assoc/arith-keys-numeric stdout"
    );
    assert_eq!(stderr, r"", "assoc/arith-keys-numeric stderr");
}

#[test]
fn assoc_arith_plus_eq() {
    // Oils `####` case: assoc/arith-plus-eq
    let (status, stdout, stderr) = run(r"declare -A A
(( A[5] = 10 ))
(( A[5] += 6 ))
echo ${A[5]}");
    assert_eq!(status, 0, "assoc/arith-plus-eq status");
    assert_eq!(
        stdout,
        r"16
",
        "assoc/arith-plus-eq stdout"
    );
    assert_eq!(stderr, r"", "assoc/arith-plus-eq stderr");
}

#[test]
fn assoc_arith_plus_eq_empty() {
    // Oils `####` case: assoc/arith-plus-eq-empty
    let (status, stdout, stderr) = run(r"declare -A A
(( A[5] += 6 ))
echo ${A[5]}");
    assert_eq!(status, 0, "assoc/arith-plus-eq-empty status");
    assert_eq!(
        stdout,
        r"6
",
        "assoc/arith-plus-eq-empty stdout"
    );
    assert_eq!(stderr, r"", "assoc/arith-plus-eq-empty stderr");
}

#[test]
fn assoc_backslash_key_arith() {
    // Oils `####` case: assoc/backslash-key-arith
    let (status, stdout, stderr) = run(r#"key='\'
declare -A A
(( A["$key"] = 42 ))
printf '<%s>\n' "${!A[@]}"
printf '<%s>\n' "${A[@]}""#);
    assert_eq!(status, 0, "assoc/backslash-key-arith status");
    assert_eq!(
        stdout,
        r"<\>
<42>
",
        "assoc/backslash-key-arith stdout"
    );
    assert_eq!(stderr, r"", "assoc/backslash-key-arith stderr");
}

#[test]
fn assoc_dquoted_lookup() {
    // Oils `####` case: assoc/dquoted-lookup
    let (status, stdout, stderr) = run(r#"declare -A a
a["aa"]=b
a["foo"]=bar
a['a+1']=c
echo "${a["aa"]}" "${a["foo"]}" "${a["a+1"]}""#);
    assert_eq!(status, 0, "assoc/dquoted-lookup status");
    assert_eq!(
        stdout,
        r"b bar c
",
        "assoc/dquoted-lookup stdout"
    );
    assert_eq!(stderr, r"", "assoc/dquoted-lookup stderr");
}

#[test]
fn assoc_dtest_v() {
    // Oils `####` case: assoc/dtest-v
    let (status, stdout, stderr) = run(r#"typeset -A assoc
assoc=([empty]='' [k]=v)
[[ -v assoc[empty] ]]
echo empty=$?
[[ -v assoc[k] ]]
echo k=$?
[[ -v assoc[nonexistent] ]]
echo nonexistent=$?
[[ -v assoc["empty"] ]]
echo empty=$?
key=empty
[[ -v assoc[$key] ]]
echo empty=$?
key=nonexistent
[[ -v assoc[$key] ]]
echo nonexistent=$?"#);
    assert_eq!(status, 0, "assoc/dtest-v status");
    assert_eq!(
        stdout,
        r"empty=0
k=0
nonexistent=1
empty=0
empty=0
nonexistent=1
",
        "assoc/dtest-v stdout"
    );
    assert_eq!(stderr, r"", "assoc/dtest-v stderr");
}

#[test]
fn assoc_dtest_v_typo() {
    // Oils `####` case: assoc/dtest-v-typo
    let (status, stdout, stderr) = run(r"typeset -A assoc
assoc=([empty]='' [k]=v)
[[ -v assoc[empty] ]]
echo empty=$?
[[ -v assoc[k] ]]
echo k=$?
[[ -v assoc[k]z ]]
echo typo=$?");
    assert_eq!(status, 0, "assoc/dtest-v-typo status");
    assert_eq!(
        stdout,
        r"empty=0
k=0
typo=1
",
        "assoc/dtest-v-typo stdout"
    );
    assert_eq!(stderr, r"", "assoc/dtest-v-typo stderr");
}

#[test]
fn assoc_elem_append() {
    // Oils `####` case: assoc/elem-append
    let (status, stdout, stderr) = run(r#"declare -A A
A['x']='foo'
A['x']+='bar'
A['x']+='bar'
printf '<%s>\n' "${A["x"]}""#);
    assert_eq!(status, 0, "assoc/elem-append status");
    assert_eq!(
        stdout,
        r"<foobarbar>
",
        "assoc/elem-append stdout"
    );
    assert_eq!(stderr, r"", "assoc/elem-append stderr");
}

#[test]
fn assoc_empty_put_get() {
    // Oils `####` case: assoc/empty-put-get
    let (status, stdout, stderr) = run(r#"declare -A A
printf '<%s>\n' "${A[@]}"
printf '<%s>\n' "${!A[@]}"
A['foo']=bar
echo ${A['foo']}"#);
    assert_eq!(status, 0, "assoc/empty-put-get status");
    assert_eq!(
        stdout,
        r"<>
<>
bar
",
        "assoc/empty-put-get stdout"
    );
    assert_eq!(stderr, r"", "assoc/empty-put-get stderr");
}

#[test]
fn assoc_empty_under_nounset() {
    // Oils `####` case: assoc/empty-under-nounset
    let (status, stdout, stderr) = run(r"set -o nounset
declare -A ASSOC=()
echo len=${#ASSOC[@]}
ASSOC['k']='32'
echo len=${#ASSOC[@]}
set +o nounset
declare -A u
echo unset len=${#u[@]}");
    assert_eq!(status, 0, "assoc/empty-under-nounset status");
    assert_eq!(
        stdout,
        r"len=0
len=1
unset len=0
",
        "assoc/empty-under-nounset stdout"
    );
    assert_eq!(stderr, r"", "assoc/empty-under-nounset stderr");
}

#[test]
fn assoc_empty_value() {
    // Oils `####` case: assoc/empty-value
    let (status, stdout, stderr) = run(r#"declare -A A=(["k"]=
)
printf '<%s>\n' "${A["k"]}""#);
    assert_eq!(status, 0, "assoc/empty-value status");
    assert_eq!(
        stdout,
        r"<>
",
        "assoc/empty-value stdout"
    );
    assert_eq!(stderr, r"", "assoc/empty-value stderr");
}

#[test]
fn assoc_implicit_increment() {
    // Oils `####` case: assoc/implicit-increment
    let (status, stdout, stderr) = run(r#"declare -a arr=( [30]=a b [40]=x y)
printf '<%s>\n' "${!arr[@]}"
printf '<%s>\n' "${arr[@]}""#);
    assert_eq!(status, 0, "assoc/implicit-increment status");
    assert_eq!(
        stdout,
        r"<30>
<31>
<40>
<41>
<a>
<b>
<x>
<y>
",
        "assoc/implicit-increment stdout"
    );
    assert_eq!(stderr, r"", "assoc/implicit-increment stderr");
}

#[test]
fn assoc_indirect_ref() {
    // Oils `####` case: assoc/indirect-ref
    let (status, stdout, stderr) = run(r#"show-values() {
  echo values: ${A[@]}
}
declare -A A=(['K']=val)
show-values
declare ref='A["K"]'
echo ref ${!ref}
key=K
declare ref='A[$key]'
echo ref ${!ref}"#);
    assert_eq!(status, 0, "assoc/indirect-ref status");
    assert_eq!(
        stdout,
        r"values: val
ref val
ref val
",
        "assoc/indirect-ref stdout"
    );
    assert_eq!(stderr, r"", "assoc/indirect-ref stderr");
}

#[test]
fn assoc_init_key_value_seq() {
    // Oils `####` case: assoc/init-key-value-seq
    let (status, stdout, stderr) = run(r"declare -A A=(1 2 3)
echo status=$?
declare -p A");
    assert_eq!(status, 0, "assoc/init-key-value-seq status");
    assert_eq!(
        stdout,
        r#"status=0
declare -A A=([3]="" [1]="2" )
"#,
        "assoc/init-key-value-seq stdout"
    );
    assert_eq!(stderr, r"", "assoc/init-key-value-seq stderr");
}

#[test]
fn assoc_keys_sorted() {
    // Oils `####` case: assoc/keys-sorted
    let (status, stdout, stderr) = run(r#"declare -A a
var='x'
a["$var"]=b
a['foo']=bar
a['a+1']=c
for key in "${!a[@]}"; do
  echo $key
done | sort"#);
    assert_eq!(status, 0, "assoc/keys-sorted status");
    assert_eq!(
        stdout,
        r"a+1
foo
x
",
        "assoc/keys-sorted stdout"
    );
    assert_eq!(stderr, r"", "assoc/keys-sorted stderr");
}

#[test]
fn assoc_length() {
    // Oils `####` case: assoc/length
    let (status, stdout, stderr) = run(r#"declare -A a
a["x"]=1
a["y"]=2
a["z"]=3
echo "${#a[@]}""#);
    assert_eq!(status, 0, "assoc/length status");
    assert_eq!(
        stdout,
        r"3
",
        "assoc/length stdout"
    );
    assert_eq!(stderr, r"", "assoc/length stderr");
}

#[test]
fn assoc_literal() {
    // Oils `####` case: assoc/literal
    let (status, stdout, stderr) = run(r#"declare -A a
a=([aa]=b [foo]=bar ['a+1']=c)
echo ${a["aa"]}
echo ${a["foo"]}
echo ${a["a+1"]}"#);
    assert_eq!(status, 0, "assoc/literal status");
    assert_eq!(
        stdout,
        r"b
bar
c
",
        "assoc/literal stdout"
    );
    assert_eq!(stderr, r"", "assoc/literal stderr");
}

#[test]
fn assoc_nameref() {
    // Oils `####` case: assoc/nameref
    let (status, stdout, stderr) = run(r#"show-values() {
  echo values: ${A[@]}
}
declare -A A=(['K']=val)
show-values
declare -n ref='A["K"]'
echo before $ref
ref='val2'
echo after $ref
show-values
echo ---
key=K
declare -n ref='A[$key]'
echo before $ref
ref='val3'
echo after $ref
show-values"#);
    assert_eq!(status, 0, "assoc/nameref status");
    assert_eq!(
        stdout,
        r"values: val
before val
after val2
values: val2
---
before val2
after val3
values: val3
",
        "assoc/nameref stdout"
    );
    assert_eq!(stderr, r"", "assoc/nameref stderr");
}

#[test]
fn assoc_patsub_values() {
    // Oils `####` case: assoc/patsub-values
    let (status, stdout, stderr) = run(r#"declare -A A
A['aa']=bbb
A['bb']=ccc
A['cc']=ddd
for val in "${A[@]//b/B}"; do
  echo $val
done | sort"#);
    assert_eq!(status, 0, "assoc/patsub-values status");
    assert_eq!(
        stdout,
        r"BBB
ccc
ddd
",
        "assoc/patsub-values stdout"
    );
    assert_eq!(stderr, r"", "assoc/patsub-values stderr");
}

#[test]
fn assoc_prefix_strip_values() {
    // Oils `####` case: assoc/prefix-strip-values
    let (status, stdout, stderr) = run(r#"declare -A A
A['aa']=one
A['bb']=two
A['cc']=three
for val in "${A[@]#t}"; do
  echo $val
done | sort"#);
    assert_eq!(status, 0, "assoc/prefix-strip-values status");
    assert_eq!(
        stdout,
        r"hree
one
wo
",
        "assoc/prefix-strip-values stdout"
    );
    assert_eq!(stderr, r"", "assoc/prefix-strip-values stderr");
}

#[test]
fn assoc_printf_v() {
    // Oils `####` case: assoc/printf-v
    let (status, stdout, stderr) = run(r#"show-values() {
  echo values: ${assoc[@]}
}
declare -A assoc=(['K']=val)
show-values
printf -v 'assoc["K"]' '/%%s/' val2
show-values
key=K
printf -v 'assoc[$key]' '/%%s/' val3
show-values"#);
    assert_eq!(status, 0, "assoc/printf-v status");
    assert_eq!(
        stdout,
        r"values: val
values: /%s/
values: /%s/
",
        "assoc/printf-v stdout"
    );
    assert_eq!(stderr, r"", "assoc/printf-v stderr");
}

#[test]
fn assoc_readonly() {
    // Oils `####` case: assoc/readonly
    let (status, stdout, stderr) = run(r"declare -Ar A
A['x']=1
echo status=$?");
    assert_eq!(status, 0, "assoc/readonly status");
    assert_eq!(
        stdout,
        r"status=1
",
        "assoc/readonly stdout"
    );
    assert_eq!(
        stderr,
        r"bash: line 2: A: readonly variable
",
        "assoc/readonly stderr"
    );
}

#[test]
fn assoc_scalar_is_zero_elem() {
    // Oils `####` case: assoc/scalar-is-zero-elem
    let (status, stdout, stderr) = run(r#"declare -A a
a=([aa]=b [foo]=bar ['a+1']=c)
echo a="${a}"
a=([0]=zzz)
echo a="${a}"
a=(['0']=yyy)
echo a="${a}""#);
    assert_eq!(status, 0, "assoc/scalar-is-zero-elem status");
    assert_eq!(
        stdout,
        r"a=
a=zzz
a=yyy
",
        "assoc/scalar-is-zero-elem stdout"
    );
    assert_eq!(stderr, r"", "assoc/scalar-is-zero-elem stderr");
}

#[test]
fn assoc_self_set_compound() {
    // Oils `####` case: assoc/self-set-compound
    let (status, stdout, stderr) = run(r#"declare -A foo
foo=(["key"]="value1")
echo ${foo["key"]}
foo=(["key"]="${foo["key"]} value2")
echo ${foo["key"]}"#);
    assert_eq!(status, 0, "assoc/self-set-compound status");
    assert_eq!(
        stdout,
        r"value1
value1 value2
",
        "assoc/self-set-compound stdout"
    );
    assert_eq!(stderr, r"", "assoc/self-set-compound stderr");
}

#[test]
fn assoc_set_to_indexed_literal() {
    // Oils `####` case: assoc/set-to-indexed-literal
    let (status, stdout, stderr) = run(r"declare -A assoc=([k1]=foo [k2]='spam eggs')
declare -p assoc
assoc=(foo 'spam eggs')
declare -p assoc");
    assert_eq!(status, 0, "assoc/set-to-indexed-literal status");
    assert_eq!(
        stdout,
        r#"declare -A assoc=([k1]="foo" [k2]="spam eggs" )
declare -A assoc=([foo]="spam eggs" )
"#,
        "assoc/set-to-indexed-literal stdout"
    );
    assert_eq!(stderr, r"", "assoc/set-to-indexed-literal stderr");
}

#[test]
fn assoc_squoted_lookup() {
    // Oils `####` case: assoc/squoted-lookup
    let (status, stdout, stderr) = run(r#"declare -A a
a["aa"]=b
a["foo"]=bar
a['a+1']=c
echo "${a['a+1']}""#);
    assert_eq!(status, 0, "assoc/squoted-lookup status");
    assert_eq!(
        stdout,
        r"c
",
        "assoc/squoted-lookup stdout"
    );
    assert_eq!(stderr, r"", "assoc/squoted-lookup stderr");
}

#[test]
fn assoc_test_v() {
    // Oils `####` case: assoc/test-v
    let (status, stdout, stderr) = run(r#"typeset -A assoc
assoc=([empty]='' [k]=v)
echo 'no quotes'
test -v assoc[empty]
echo empty=$?
test -v assoc[k]
echo k=$?
test -v assoc[nonexistent]
echo nonexistent=$?
echo
test -v assoc["empty"]
echo empty=$?
test -v assoc['k']
echo k=$?
test -v assoc['nonexistent']
echo nonexistent=$?"#);
    assert_eq!(status, 0, "assoc/test-v status");
    assert_eq!(
        stdout,
        r"no quotes
empty=0
k=0
nonexistent=1

empty=0
k=0
nonexistent=1
",
        "assoc/test-v stdout"
    );
    assert_eq!(stderr, r"", "assoc/test-v stderr");
}

#[test]
fn assoc_test_v_dynamic() {
    // Oils `####` case: assoc/test-v-dynamic
    let (status, stdout, stderr) = run(r"typeset -A assoc
assoc=([empty]='' [k]=v)
key=empty
test -v 'assoc[$key]'
echo empty=$?
key=k
test -v 'assoc[$key]'
echo k=$?
key=nonexistent
test -v 'assoc[$key]'
echo nonexistent=$?");
    assert_eq!(status, 0, "assoc/test-v-dynamic status");
    assert_eq!(
        stdout,
        r"empty=0
k=0
nonexistent=1
",
        "assoc/test-v-dynamic stdout"
    );
    assert_eq!(stderr, r"", "assoc/test-v-dynamic stderr");
}

#[test]
fn assoc_unquoted_arith_key() {
    // Oils `####` case: assoc/unquoted-arith-key
    let (status, stdout, stderr) = run(r#"declare -A a
a["aa"]=b
a["foo"]=bar
a['a+1']=c
echo "${a[a+1]}""#);
    assert_eq!(status, 0, "assoc/unquoted-arith-key status");
    assert_eq!(
        stdout,
        r"c
",
        "assoc/unquoted-arith-key stdout"
    );
    assert_eq!(stderr, r"", "assoc/unquoted-arith-key stderr");
}

#[test]
fn assoc_unquoted_key_var() {
    // Oils `####` case: assoc/unquoted-key-var
    let (status, stdout, stderr) = run(r#"declare -A A
A["aa"]=b
A["foo"]=bar
key=foo
echo ${A[$key]}
i=a
echo ${A["$i$i"]}"#);
    assert_eq!(status, 0, "assoc/unquoted-key-var status");
    assert_eq!(
        stdout,
        r"bar
b
",
        "assoc/unquoted-key-var stdout"
    );
    assert_eq!(stderr, r"", "assoc/unquoted-key-var stderr");
}

#[test]
fn assoc_unset_key_forms() {
    // Oils `####` case: assoc/unset-key-forms
    let (status, stdout, stderr) = run(r#"show-len() {
  echo len=${#assoc[@]}
}
declare -A assoc=(['K']=val)
show-len
unset -v 'assoc["K"]'
show-len
declare -A assoc=(['K']=val)
show-len
key=K
unset -v 'assoc[$key]'
show-len
declare -A assoc=(['K']=val)
show-len
unset -v 'assoc[$(echo K)]'
show-len"#);
    assert_eq!(status, 0, "assoc/unset-key-forms status");
    assert_eq!(
        stdout,
        r"len=1
len=0
len=1
len=0
len=1
len=0
",
        "assoc/unset-key-forms stdout"
    );
    assert_eq!(stderr, r"", "assoc/unset-key-forms stderr");
}

#[test]
fn assoc_values_sorted() {
    // Oils `####` case: assoc/values-sorted
    let (status, stdout, stderr) = run(r#"declare -A A
var='x'
A["$var"]=b
A['foo']=bar
A['a+1']=c
for val in "${A[@]}"; do
  echo $val
done | sort"#);
    assert_eq!(status, 0, "assoc/values-sorted status");
    assert_eq!(
        stdout,
        r"b
bar
c
",
        "assoc/values-sorted stdout"
    );
    assert_eq!(stderr, r"", "assoc/values-sorted stderr");
}

#[test]
fn assoc_zero_string_key() {
    // Oils `####` case: assoc/zero-string-key
    let (status, stdout, stderr) = run(r#"declare -A a
a["0"]=a
a["1"]=b
a["2"]=c
echo 0 "${a[0]}" 1 "${a[1]}" 2 "${a[2]}""#);
    assert_eq!(status, 0, "assoc/zero-string-key status");
    assert_eq!(
        stdout,
        r"0 a 1 b 2 c
",
        "assoc/zero-string-key stdout"
    );
    assert_eq!(stderr, r"", "assoc/zero-string-key stderr");
}
