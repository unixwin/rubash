use std::process::{Command, Output};

fn run(source: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", source])
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash")
}

fn assert_output(source: &str, expected: &str, bad_subscripts: usize) {
    let output = run(source);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.lines().count(), bad_subscripts, "{stderr}");
    assert!(
        stderr
            .lines()
            .all(|line| line.ends_with(": bad array subscript")),
        "{stderr}"
    );
}

#[test]
fn getopts_stale_optind_clamps_before_shift_in_repeated_function_calls() {
    assert_output(
        r#"f(){ while getopts ":ab:" option; do case $option in a) echo got_a;; b) echo got_b=$OPTARG;; esac; done; echo "after optind=$OPTIND"; shift $((OPTIND-1)); echo "rest=$*"; }; f -a -b X; f -z; f -b"#,
        "got_a\ngot_b=X\nafter optind=4\nrest=\nafter optind=2\nrest=\nafter optind=2\nrest=\n",
        0,
    );
}

#[test]
fn getopts_stale_explicit_empty_and_clustered_arguments_reset_at_eof() {
    assert_output(
        r#"OPTIND=99; getopts :a option -a; printf '%s:%s:%s:%s\n' "$?" "$OPTIND" "$option" "${OPTARG-unset}"; OPTIND=99; set --; getopts :a option; printf '%s:%s:%s:%s\n' "$?" "$OPTIND" "$option" "${OPTARG-unset}"; OPTIND=1; getopts ab option -ab; OPTIND=99; getopts ab option -a; printf '%s:%s:%s\n' "$?" "$OPTIND" "$option"; OPTIND=1; getopts ab option -ab; printf '%s:%s:%s\n' "$?" "$OPTIND" "$option""#,
        "1:2:?:unset\n1:1:?:unset\n1:2:?\n0:1:a\n",
        0,
    );
}

#[test]
fn negative_array_subscripts_only_report_each_out_of_bounds_occurrence() {
    assert_output(
        r#"a=(aa bb cc); echo "${a[-1]}" "${a[-3]}" "${a[-4]-oob}"; echo "${a[-1]} ${a[-2]}"; echo "${a[-4]-left}" "${a[-4]-right}""#,
        "cc aa oob\ncc bb\nleft right\n",
        3,
    );
}

#[test]
fn sparse_negative_array_subscripts_use_max_index_and_emit_one_diagnostic() {
    assert_output(
        r#"declare -a sparse=([2]=two [9]=nine); echo "${sparse[-1]}" "${sparse[-8]}" "${sparse[-11]-oob}""#,
        "nine two oob\n",
        1,
    );
}

#[test]
fn array_subscript_side_effects_are_once_per_fragment_not_once_per_command() {
    assert_output(
        r#"a=(aa bb cc); index=-4; echo "${a[index++]-oob}" "index=$index"; index=-4; echo "${a[index++]-oob}" "${a[index++]-oob}" "index=$index"; index=0; echo "${a[index++]-fallback}" "${a[index++]-fallback}" "index=$index"; index=0; echo "${a[index++]}:${a[index++]}" "index=$index"; index=-4; echo "${a[index++]-$@}" "index=$index""#,
        "oob index=-3\noob aa index=-2\naa bb index=2\naa:bb index=2\n index=-3\n",
        3,
    );
}
