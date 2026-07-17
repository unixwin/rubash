use super::super::*;
use std::fs;

#[test]
fn test_indirect_parameter_uses_positional_parameter_name() {
    let output_path = "target/rubash-param-indirect-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function p {{ target=value; echo ${{!1}} > {output_path}; }}; p target");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_prefix_expands_matching_variable_names() {
    let output_path = "target/rubash-param-indirect-prefix-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RUBASH_INDIRECT_A=1; RUBASH_INDIRECT_B=2; echo ${{!RUBASH_INDIRECT_*}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "RUBASH_INDIRECT_A RUBASH_INDIRECT_B\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_indirect_prefix_expansion_splits_names() {
    let output_path = "target/rubash-param-indirect-prefix-split-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RUBASH_INDIRECT_SPLIT_A=1; RUBASH_INDIRECT_SPLIT_B=2; \
         printf '<%s>\\n' ${{!RUBASH_INDIRECT_SPLIT_*}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<RUBASH_INDIRECT_SPLIT_A>\n<RUBASH_INDIRECT_SPLIT_B>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_indirect_prefix_at_expansion_splits_names() {
    let output_path = "target/rubash-param-indirect-prefix-quoted-at-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RUBASH_INDIRECT_QUOTED_A=1; RUBASH_INDIRECT_QUOTED_B=2; \
         printf 'at<%s>\\n' \"${{!RUBASH_INDIRECT_QUOTED_@}}\" > {output_path}; \
         printf 'star<%s>\\n' \"${{!RUBASH_INDIRECT_QUOTED_*}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "at<RUBASH_INDIRECT_QUOTED_A>\nat<RUBASH_INDIRECT_QUOTED_B>\nstar<RUBASH_INDIRECT_QUOTED_A RUBASH_INDIRECT_QUOTED_B>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_indirect_array_indices_split_words() {
    let output_path = "target/rubash-param-indirect-array-indices-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("arr=([2]=two [5]=five); printf '[%s]\\n' ${{!arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "[2]\n[5]\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_array_parameter_transform_expands_first_value() {
    let output_path = "target/rubash-param-indirect-array-transform-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("arr=(alpha beta); ref=arr; echo ${{!ref[@]@Q}} ${{!ref[*]@U}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "'alpha' ALPHA\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_assignment_transform_matches_at_and_star_forms() {
    let output_path = "target/rubash-param-array-assignment-transform-output.txt";
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_ASSIGN_TRANSFORM_ARRAY",
        "RUBASH_ASSIGN_TRANSFORM_ASSOC",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_ASSIGN_TRANSFORM_ARRAY=(zero one); declare -A RUBASH_ASSIGN_TRANSFORM_ASSOC=([0]=z); \
         echo arr_star:${{RUBASH_ASSIGN_TRANSFORM_ARRAY[*]@A}} > {output_path}; \
         echo arr_at:${{RUBASH_ASSIGN_TRANSFORM_ARRAY[@]@A}} >> {output_path}; \
         echo assoc_scalar:${{RUBASH_ASSIGN_TRANSFORM_ASSOC@A}} >> {output_path}; \
         echo assoc_at:${{RUBASH_ASSIGN_TRANSFORM_ASSOC[@]@A}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "arr_star:declare -a RUBASH_ASSIGN_TRANSFORM_ARRAY=([0]=\"zero\" [1]=\"one\")\narr_at:declare -a RUBASH_ASSIGN_TRANSFORM_ARRAY=([0]=\"zero\" [1]=\"one\")\nassoc_scalar:declare -A RUBASH_ASSIGN_TRANSFORM_ASSOC='z'\nassoc_at:declare -A RUBASH_ASSIGN_TRANSFORM_ASSOC=([0]=\"z\" )\n"
    );
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_ASSIGN_TRANSFORM_ARRAY",
        "RUBASH_ASSIGN_TRANSFORM_ASSOC",
    ] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_array_value_parameter_transforms_apply_to_elements() {
    let output_path = "target/rubash-param-array-value-transform-output.txt";
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_VALUE_TRANSFORM_ARRAY",
        "RUBASH_VALUE_TRANSFORM_ASSOC",
        "RUBASH_VALUE_TRANSFORM_REF",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_VALUE_TRANSFORM_ARRAY=('a b' c); \
         declare -A RUBASH_VALUE_TRANSFORM_ASSOC=([k]='v w' [0]=z); \
         declare -n RUBASH_VALUE_TRANSFORM_REF=RUBASH_VALUE_TRANSFORM_ARRAY; \
         echo arr_q:${{RUBASH_VALUE_TRANSFORM_ARRAY@Q}} > {output_path}; \
         echo arr0_q:${{RUBASH_VALUE_TRANSFORM_ARRAY[0]@Q}} >> {output_path}; \
         echo arr0_u:${{RUBASH_VALUE_TRANSFORM_ARRAY[0]@U}} >> {output_path}; \
         echo assoc_q:${{RUBASH_VALUE_TRANSFORM_ASSOC@Q}} >> {output_path}; \
         echo assoc_k_q:${{RUBASH_VALUE_TRANSFORM_ASSOC[k]@Q}} >> {output_path}; \
         echo ref_q:${{RUBASH_VALUE_TRANSFORM_REF@Q}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "arr_q:'a b'\narr0_q:'a b'\narr0_u:A B\nassoc_q:'z'\nassoc_k_q:'v w'\nref_q:'a b'\n"
    );
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_VALUE_TRANSFORM_ARRAY",
        "RUBASH_VALUE_TRANSFORM_ASSOC",
        "RUBASH_VALUE_TRANSFORM_REF",
    ] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_parameter_upper_first_transform_applies_to_values() {
    let output_path = "target/rubash-param-upper-first-transform-output.txt";
    let _ = fs::remove_file(output_path);
    for name in ["RUBASH_UPPER_FIRST_ARRAY", "RUBASH_UPPER_FIRST_REF"] {
        std::env::remove_var(name);
    }
    let input = format!(
        "v=alpha; RUBASH_UPPER_FIRST_ARRAY=(alpha beta); \
         declare -n RUBASH_UPPER_FIRST_REF=RUBASH_UPPER_FIRST_ARRAY; \
         echo scalar:${{v@u}} > {output_path}; \
         echo elem:${{RUBASH_UPPER_FIRST_ARRAY[1]@u}} >> {output_path}; \
         echo arr:${{RUBASH_UPPER_FIRST_ARRAY[@]@u}} >> {output_path}; \
         echo ref:${{RUBASH_UPPER_FIRST_REF@u}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "scalar:Alpha\nelem:Beta\narr:Alpha Beta\nref:Alpha\n"
    );
    let _ = fs::remove_file(output_path);
    for name in ["RUBASH_UPPER_FIRST_ARRAY", "RUBASH_UPPER_FIRST_REF"] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_parameter_prompt_transform_expands_version_escapes() {
    let output_path = "target/rubash-param-prompt-transform-version-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("p='v=\\v V=\\V s=\\s'; echo \"${{p@P}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    let release = env!("CARGO_PKG_VERSION");
    let mut parts = release.split('.');
    let short = format!(
        "{}.{}",
        parts.next().unwrap_or("0"),
        parts.next().unwrap_or("0")
    );
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("v={short} V={release} s=bash\n")
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_prompt_transform_expands_job_count_escape() {
    let output_path = "target/rubash-param-prompt-transform-jobs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("p='jobs=\\j'; echo \"${{p@P}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "jobs=0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_prompt_transform_expands_time_escapes() {
    let output_path = "target/rubash-param-prompt-transform-time-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("p='t=\\t T=\\T at=\\@ A=\\A'; echo \"${{p@P}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let pattern = regex::Regex::new(
        r"^t=\d{2}:\d{2}:\d{2} T=\d{2}:\d{2}:\d{2} at=\d{2}:\d{2} (AM|PM) A=\d{2}:\d{2}\n$",
    )
    .unwrap();
    assert!(pattern.is_match(&output), "{output:?}");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_prompt_transform_expands_date_escapes() {
    let output_path = "target/rubash-param-prompt-transform-date-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("p='d=\\d D=\\D{{%Y-%m}}'; echo \"${{p@P}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let pattern =
        regex::Regex::new(r"^d=[A-Z][a-z]{2} [A-Z][a-z]{2} [ 0-9][0-9] D=\d{4}-\d{2}\n$").unwrap();
    assert!(pattern.is_match(&output), "{output:?}");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_prompt_transform_expands_octal_escapes() {
    let output_path = "target/rubash-param-prompt-transform-octal-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("p='x=\\141 y=\\060 end=\\0123'; printf '<%s>\\n' \"${{p@P}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<x=a y=0 end=\n3>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_array_pattern_removes_prefixes_and_suffixes() {
    let output_path = "target/rubash-param-indirect-array-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(aaa bbb); ref='arr[@]'; echo ${{!ref##aa}} > {output_path}; echo ${{!ref[@]%b}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a bbb\naaa bb\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_array_star_pattern_and_case_use_ifs_first_char() {
    let output_path = "target/rubash-param-indirect-array-star-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "IFS=,; arr=(alpha 'two words'); target='arr[*]'; \
         printf 'case<%s>\\n' \"${{!target^^}}\" > {output_path}; \
         printf 'pat<%s>\\n' \"${{!target#a}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "case<ALPHA,TWO WORDS>\npat<lpha,two words>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_replacement_expands_target_values() {
    let output_path = "target/rubash-param-indirect-replacement-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=banana; ref=value; printf 'scalar<%s>\\n' \"${{!ref//a/o}}\" > {output_path}; \
         IFS=,; arr=(banana gamma); target='arr[*]'; \
         printf 'array<%s>\\n' \"${{!target//a/o}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "scalar<bonono>\narray<bonono,gommo>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_substring_expands_target_values() {
    let output_path = "target/rubash-param-indirect-substring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abcdef; ref=value; printf 'scalar<%s>\\n' \"${{!ref:1:3}}\" > {output_path}; \
         IFS=,; arr=(zero one two); star='arr[*]'; at='arr[@]'; \
         printf 'star<%s>\\n' \"${{!star:1:2}}\" >> {output_path}; \
         printf 'at<%s>\\n' \"${{!at:1:2}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "scalar<bcd>\nstar<one,two>\nat<one>\nat<two>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_word_operators_expand_target_values() {
    let output_path = "target/rubash-param-indirect-word-operators-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=ok; ref=value; printf 'set<%s>\\n' \"${{!ref:-fallback}}\" > {output_path}; \
         unset value; printf 'default<%s>\\n' \"${{!ref:-fallback}}\" >> {output_path}; \
         printf 'assign<%s>\\n' \"${{!ref:=assigned}}\" >> {output_path}; \
         printf 'value<%s>\\n' \"$value\" >> {output_path}; \
         printf 'alt<%s>\\n' \"${{!ref:+alternate}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "set<ok>\ndefault<fallback>\nassign<assigned>\nvalue<assigned>\nalt<alternate>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_array_element_target_expands_value() {
    let output_path = "target/rubash-param-indirect-array-element-target-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(banana gamma); ref='arr[0]'; \
         printf 'plain<%s>\\n' \"${{!ref}}\" > {output_path}; \
         printf 'replace<%s>\\n' \"${{!ref//a/o}}\" >> {output_path}; \
         printf 'substr<%s>\\n' \"${{!ref:1:3}}\" >> {output_path}; \
         printf 'default<%s>\\n' \"${{!ref:-fallback}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "plain<banana>\nreplace<bonono>\nsubstr<ana>\ndefault<banana>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_test_v_checks_array_subscripts() {
    let output_path = "target/rubash-test-v-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=value; arr=(zero one two); i=1; declare -A assoc; assoc[one]=alpha; \
         test -v 'v[0]'; echo $? > {output_path}; \
         test -v 'v[1]'; echo $? >> {output_path}; \
         test -v 'arr[i+1]'; echo $? >> {output_path}; \
         test -v 'arr[-1]'; echo $? >> {output_path}; \
         test -v 'arr[9]'; echo $? >> {output_path}; \
         test -v 'assoc[one]'; echo $? >> {output_path}; \
         test -v 'assoc[two]'; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n0\n1\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_v_checks_array_subscripts() {
    let output_path = "target/rubash-conditional-v-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=value; arr=(zero one two); i=1; declare -A assoc; assoc[one]=alpha; \
         [[ -v v[0] ]]; echo $? > {output_path}; \
         [[ -v v[1] ]]; echo $? >> {output_path}; \
         [[ -v arr[i+1] ]]; echo $? >> {output_path}; \
         [[ -v arr[-1] ]]; echo $? >> {output_path}; \
         [[ -v arr[9] ]]; echo $? >> {output_path}; \
         [[ -v assoc[one] ]]; echo $? >> {output_path}; \
         [[ -v assoc[two] ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n0\n1\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_v_array_whole_subscript_checks_indexed_array_elements() {
    let output_path = "target/rubash-v-array-whole-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one); empty=(); declare -A assoc; assoc[k]=v; \
         [[ -v arr[@] ]]; echo cond_arr_at:$? > {output_path}; \
         [[ -v arr[*] ]]; echo cond_arr_star:$? >> {output_path}; \
         [[ -v empty[@] ]]; echo cond_empty:$? >> {output_path}; \
         [[ -v assoc[@] ]]; echo cond_assoc:$? >> {output_path}; \
         test -v 'arr[@]'; echo test_arr_at:$? >> {output_path}; \
         test -v 'empty[@]'; echo test_empty:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "cond_arr_at:0\ncond_arr_star:0\ncond_empty:1\ncond_assoc:1\ntest_arr_at:0\ntest_empty:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_v_bare_array_name_checks_zero_element() {
    let output_path = "target/rubash-v-bare-array-name-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one); sparse=([2]=two); empty=(); \
         declare -A assoc_zero=([0]=zero [k]=v); declare -A assoc_key=([k]=v); \
         [[ -v arr ]]; echo cond_arr:$? > {output_path}; \
         [[ -v sparse ]]; echo cond_sparse:$? >> {output_path}; \
         [[ -v empty ]]; echo cond_empty:$? >> {output_path}; \
         [[ -v assoc_zero ]]; echo cond_assoc_zero:$? >> {output_path}; \
         [[ -v assoc_key ]]; echo cond_assoc_key:$? >> {output_path}; \
         test -v arr; echo test_arr:$? >> {output_path}; \
         test -v sparse; echo test_sparse:$? >> {output_path}; \
         test -v assoc_zero; echo test_assoc_zero:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "cond_arr:0\ncond_sparse:1\ncond_empty:1\ncond_assoc_zero:0\ncond_assoc_key:1\ntest_arr:0\ntest_sparse:1\ntest_assoc_zero:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_v_expands_dynamic_operand() {
    let output_path = "target/rubash-conditional-v-dynamic-operand-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero); name='arr[0]'; declare -A assoc; assoc[one]=1; key=one; operand='assoc[one]'; \
         [[ -v $name ]]; echo indexed_name:$? > {output_path}; \
         [[ -v assoc[$key] ]]; echo assoc_key:$? >> {output_path}; \
         [[ -v $operand ]]; echo assoc_operand:$? >> {output_path}; \
         [[ -v name ]]; echo plain_name:$? >> {output_path}; \
         [[ -v missing[$key] ]]; echo missing_assoc:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "indexed_name:0\nassoc_key:0\nassoc_operand:0\nplain_name:0\nmissing_assoc:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_r_checks_nameref_variables() {
    let output_path = "target/rubash-conditional-nameref-unary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "target=value; declare -n ref=target; readonly ro=value; \
         [[ -R ref ]]; echo $? > {output_path}; \
         [[ -R target ]]; echo $? >> {output_path}; \
         [[ -R ro ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_r_expands_dynamic_operand() {
    let output_path = "target/rubash-conditional-nameref-dynamic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "target=value; declare -n ref=target; name=ref; plain=target; \
         [[ -R $name ]]; echo dynamic_ref:$? > {output_path}; \
         [[ -R $plain ]]; echo dynamic_plain:$? >> {output_path}; \
         [[ -R name ]]; echo literal_name:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "dynamic_ref:0\ndynamic_plain:1\nliteral_name:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_string_unary_checks_expanded_value() {
    let output_path = "target/rubash-conditional-string-unary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc; empty=; [[ -n abc ]]; echo $? > {output_path}; [[ -n $empty ]]; echo $? >> {output_path}; [[ -z abc ]]; echo $? >> {output_path}; [[ -z $empty ]]; echo $? >> {output_path}; [[ -n $value ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n0\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_binary_checks_expand_values() {
    let output_path = "target/rubash-conditional-binary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "left=abc; right=def; n=3; [[ $left = abc ]]; echo $? > {output_path}; [[ $left != $right ]]; echo $? >> {output_path}; [[ $n -ne 4 ]]; echo $? >> {output_path}; [[ $n -lt 4 ]]; echo $? >> {output_path}; [[ $n -le 3 ]]; echo $? >> {output_path}; [[ $n -ge 3 ]]; echo $? >> {output_path}; [[ $n -gt 4 ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n0\n0\n0\n0\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}
