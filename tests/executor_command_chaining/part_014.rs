use super::super::*;
use std::fs;

#[test]
fn test_bash_aliases_preserves_values_with_spaces() {
    let output_path = "target/rubash-bash-aliases-spaces-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "alias qux='/usr/local/bin/qux -l'; \
         BASH_ALIASES[blat]='cd /blat ; echo $PWD'; \
         printf '%s\\n' \"${{BASH_ALIASES[qux]}}\" \"${{BASH_ALIASES[blat]}}\" > {output_path}; \
         declare -p BASH_ALIASES >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/usr/local/bin/qux -l\ncd /blat ; echo $PWD\ndeclare -A BASH_ALIASES=([blat]=\"cd /blat ; echo \\$PWD\" [qux]=\"/usr/local/bin/qux -l\" )\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unset_indexed_array_element() {
    let output_path = "target/rubash-unset-array-element-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); unset 'arr[1]'; echo ${{!arr[@]}} / ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0 2 / zero two\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unset_indexed_array_negative_subscript() {
    let output_path = "target/rubash-unset-array-negative-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); unset 'arr[-1]'; echo ${{!arr[@]}} / ${{arr[@]}} > {output_path}; \
         arr=(zero one two); unset 'arr[-2]'; echo ${{!arr[@]}} / ${{arr[@]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 1 / zero one\n0 2 / zero two\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unset_indexed_array_arithmetic_subscript() {
    let output_path = "target/rubash-unset-array-arith-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); alen=${{#arr[@]}}; unset 'arr[$alen-1]'; echo ${{!arr[@]}} / ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0 1 / zero one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indexed_array_arithmetic_subscript_assignment_and_expansion() {
    let output_path = "target/rubash-array-arith-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); i=1; arr[i]=ONE; arr[i+1]+=!; printf '%s/%s/%s\\n' \"${{arr[i]}}\" \"${{arr[i+1]}}\" \"${{arr[-1]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ONE/two!/two!\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indexed_array_assignment_preserves_empty_and_sparse_elements() {
    let output_path = "target/rubash-indexed-array-sparse-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(); arr[0]=; arr[2]=two; arr[2]+=!; \
         printf '<%s>|<%s>|<%s>|%s\\n' \"${{arr[0]}}\" \"${{arr[1]-missing}}\" \"${{arr[2]}}\" \"${{!arr[@]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<>|<missing>|<two!>|0\n<2>|<>|<>|\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_eval_multiple_indexed_array_element_assignments() {
    let output_path = "target/rubash-eval-multi-array-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -a arr; eval \"arr[0]=zero arr[1]=one\"; printf '%s %s\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "zero one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_parameter_compound_array_assignment_preserves_quote_chars() {
    let output_path = "target/rubash-array-param-quotes-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command='String \"\" validateAndParse NaN'; words=($command); printf '<%s>|<%s>|<%s>|<%s>\\n' \"${{words[0]}}\" \"${{words[1]}}\" \"${{words[2]}}\" \"${{words[3]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<String>|<\"\">|<validateAndParse>|<NaN>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_array_star_assignment_preserves_empty_quote_argument_for_eval() {
    let output_path = "target/rubash-array-star-eval-quotes-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "capture() {{ printf '<%s>|<%s>|<%s>\\n' \"$1\" \"$2\" \"$3\" > {output_path}; }}; \
         command='String \"\" validateAndParse'; words=($command); words[0]=capture; full=\"${{words[*]}}\"; eval \"$full\""
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<>|<validateAndParse>|<>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_indexed_array_assignment_preserves_explicit_indices() {
    let output_path = "target/rubash-indexed-array-compound-sparse-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=([2]=two [0]=zero middle); arr+=([5]=five tail); \
         printf '%s / %s\\n' \"${{!arr[*]}}\" \"${{arr[*]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 1 2 5 6 / zero middle two five tail\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_indexed_array_assignment_resolves_negative_indices() {
    let output_path = "target/rubash-indexed-array-compound-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=([2]=two [5]=five); arr+=([-1]=FIVE [-4]=TWO); \
         printf '%s / %s\\n' \"${{!arr[*]}}\" \"${{arr[*]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 5 / TWO FIVE\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_indexed_array_assignment_evaluates_arithmetic_indices() {
    let output_path = "target/rubash-indexed-array-compound-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "one= two=; arr=([one]=first [two]=second [2*3]=six); \
         printf '%s / %s / %s\\n' \"${{arr[one]}}\" \"${{arr[two]}}\" \"${{arr[6]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "second / second / six\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indexed_compound_append_uses_string_append_for_plain_arrays() {
    let output_path = "target/rubash-indexed-compound-string-append-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=([0]=a); arr+=([0]+=b [2]+=z); \
         printf '%s / %s / %s\\n' \"${{arr[0]}}\" \"${{arr[2]}}\" \"${{!arr[*]}}\" > {output_path}; \
         declare -a darr=([0]=a); declare -a darr+=([0]+=b); \
         printf '%s\\n' \"${{darr[0]}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ab / z / 0 2\nab\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_integer_indexed_compound_append_uses_arithmetic_append() {
    let output_path = "target/rubash-indexed-compound-integer-append-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -i -a arr=([0]=1); arr+=([0]+=2 [2]+=5); \
         printf '%s / %s\\n' \"${{arr[0]}}\" \"${{arr[2]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 / 5\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_indexed_array_assignment_preserves_quoted_words() {
    let output_path = "target/rubash-indexed-array-compound-quotes-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(one \"two words\" [4]=\"four words\"); \
         printf '<%s>/<%s>/<%s>\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" \"${{arr[4]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<one>/<two words>/<four words>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_array_assignment_splits_unquoted_command_substitution() {
    let output_path = "target/rubash-array-command-subst-split-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(a $(printf 'b c\\n') d); \
         printf '<%s>\\n' \"${{arr[@]}}\" > {output_path}; \
         arr=($(printf 'one two\\n')); \
         printf 'len:%s:<%s>:<%s>\\n' \"${{#arr[@]}}\" \"${{arr[0]}}\" \"${{arr[1]}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b>\n<c>\n<d>\nlen:2:<one>:<two>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_array_assignment_globs_unquoted_command_substitution_fields() {
    let dir_path = target_test_path("rubash-array-command-subst-glob");
    let output_path = target_test_path("rubash-array-command-subst-glob-output.txt");
    let shell_dir_path = shell_test_path(&dir_path);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_dir_all(&dir_path);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&dir_path).unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    let input = format!(
        "cd {shell_dir_path}; touch a.rs b.rs c.txt; \
         arr=($(printf '%s\\n' '*.rs')); \
         printf '%s:<%s>:<%s>\\n' \"${{#arr[@]}}\" \"${{arr[0]}}\" \"${{arr[1]}}\" > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    std::env::set_current_dir(old_cwd).unwrap();
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "2:<a.rs>:<b.rs>\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_compound_array_assignment_preserves_quoted_command_substitution() {
    let output_path = "target/rubash-array-command-subst-quoted-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(\"$(printf 'b c\\n')\"); \
         printf 'len:%s:<%s>\\n' \"${{#arr[@]}}\" \"${{arr[0]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "len:1:<b c>\n");
    let _ = fs::remove_file(output_path);
}
