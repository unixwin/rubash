use std::process::Command;

fn run_script(script: &str, args: &[&str]) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue4248-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(args).arg("case.sh").current_dir(&dir);
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_script(script: &str, expected: &str) {
    let (stdout, stderr, code) = run_script(script, &[]);
    assert_eq!(stdout, expected);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

const MATRIX_SCRIPT: &str = r##"showf() {
  printf 'n=%s' "${#f[@]}"
  i=0; while [ "$i" -lt "${#f[@]}" ]; do printf ' [%s]=<%s>' "$i" "${f[$i]}"; i=$((i+1)); done
  printf '\n'
}
showa() { printf 'A[k]=<%s>\n' "${A[k]}"; }
printf 'c1 '; unset x f; f=(${x:-$'a;b'}); showf
printf 'c2 '; unset x f; f=(${x:-$'abc'}); showf
printf 'c3 '; unset x f; f=(${x:-$'a\tb'}); showf
printf 'c4 '; unset x f; f=(${x:-$'a b'}); showf
printf 'c5 '; unset x f; f=("${x:-$'a;b'}"); showf
printf 'c6 '; unset x A; declare -A A=([k]=${x:-$'a;b'}); showa
printf 'c7 '; unset x f; f=([0]=${x:-$'a;b'}); showf
printf 'c8 '; unset x f; f=(${x:-'a;b'} ${x:-"c;d"} ${x:-$'e\'f'}); showf
printf 'c9 '; unset x y f; f=(${y:=$'p\nq'} ${y:+$'r;s'}); printf 'y=<%s> ' "$y"; showf
printf 'c10 '; unset x f; f=(${x:-a $'b c'}); showf
printf 'c11 '; unset x d; declare -a d=(${x:-$'a;b'}); printf 'dn=%s d0=<%s>\n' "${#d[@]}" "${d[0]}"
printf 'xset1 '; x=set f=; f=(${x:-$'a;b'}); showf
printf 'xempty1 '; x= f=; f=(${x-$'a;b'}); showf
printf 'altset '; x=set f=; f=(${x:+$'a;b'}); showf
printf 'altunset '; unset x f; f=(${x:+$'a;b'}); showf
printf 'scalar '; unset x s; s=${x:-$'a;b'}; printf '<%s>\n' "$s"
printf 'sqpat '; x='a;b' f=; f=(${x#$'a'}); showf
printf 'rem '; x='a;b' f=; f=(${x%$';b'}); showf
printf 'sub '; x='a;b' f=; f=(${x/$'a'/$'b'}); showf
"##;

const MATRIX_EXPECTED: &str = "c1 n=1 [0]=<a;b>\n\
c2 n=1 [0]=<abc>\n\
c3 n=1 [0]=<a\tb>\n\
c4 n=1 [0]=<a b>\n\
c5 n=1 [0]=<a;b>\n\
c6 A[k]=<a;b>\n\
c7 n=1 [0]=<a;b>\n\
c8 n=3 [0]=<a;b> [1]=<c;d> [2]=<e'f>\n\
c9 y=<p\n\
q> n=3 [0]=<p> [1]=<q> [2]=<r;s>\n\
c10 n=2 [0]=<a> [1]=<b c>\n\
c11 dn=1 d0=<a;b>\n\
xset1 n=1 [0]=<set>\n\
xempty1 n=0\n\
altset n=1 [0]=<a;b>\n\
altunset n=0\n\
scalar <a;b>\n\
sqpat n=1 [0]=<;b>\n\
rem n=1 [0]=<a>\n\
sub n=1 [0]=<b;b>\n";

#[test]
fn compound_ansic_operator_rhs_matrix() {
    assert_script(MATRIX_SCRIPT, MATRIX_EXPECTED);
}

#[test]
fn compound_ansic_decoded_syntax_bytes_are_data() {
    assert_script(
        "unset x f\nf=(${x:-$'a\\'b'})\nprintf '<%s> n=%s\\n' \"${f[0]}\" \"${#f[@]}\"\nf=(${x:-$'a\"b'})\nprintf '<%s> n=%s\\n' \"${f[0]}\" \"${#f[@]}\"\nf=(${x:-$'a\\\\b'})\nprintf '<%s> n=%s\\n' \"${f[0]}\" \"${#f[@]}\"\n",
        "<a'b> n=1\n<a\"b> n=1\n<a\\b> n=1\n",
    );
}

#[test]
fn posix_double_quoted_operator_ansic_is_literal() {
    let (stdout, stderr, code) = run_script(
        "unset x f\nf=(\"${x:-$'a;b'}\")\nprintf 'dq <%s>\\n' \"${f[0]}\"\nf=(${x:-$'a;b'})\nprintf 'uq <%s>\\n' \"${f[0]}\"\n",
        &["--posix"],
    );
    assert_eq!(stdout, "dq <$'a;b'>\nuq <a;b>\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
