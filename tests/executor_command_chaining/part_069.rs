use super::super::*;
use std::fs;

#[test]
fn test_declare_plus_array_attributes_cannot_destroy_arrays() {
    let output_path = target_test_path("rubash-declare-plus-array-attrs-output.txt");
    let error_path = target_test_path("rubash-declare-plus-array-attrs-error.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let input = format!(
        "declare -a a=(x y); declare +a a 2> {shell_error_path}; echo indexed:$? > {shell_output_path}; declare -p a >> {shell_output_path}; \
         declare -A A=([k]=v); declare +A A 2>> {shell_error_path}; echo assoc:$? >> {shell_output_path}; declare -p A >> {shell_output_path}; \
         declare +A a; echo cross-indexed:$? >> {shell_output_path}; declare -p a >> {shell_output_path}; \
         declare +a A; echo cross-assoc:$? >> {shell_output_path}; declare -p A >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "indexed:1\ndeclare -a a=([0]=\"x\" [1]=\"y\")\nassoc:1\ndeclare -A A=([k]=\"v\" )\ncross-indexed:0\ndeclare -a a=([0]=\"x\" [1]=\"y\")\ncross-assoc:0\ndeclare -A A=([k]=\"v\" )\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("declare: a: cannot destroy array variables in this way"));
    assert!(error.contains("declare: A: cannot destroy array variables in this way"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_declare_nameref_reads_and_assigns_target() {
    let output_path = target_test_path("rubash-declare-nameref-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    std::env::remove_var("RUBASH_NAMEREF_TARGET");
    std::env::remove_var("RUBASH_NAMEREF_REF");
    std::env::remove_var("RUBASH_NAMEREF_REF2");
    let input = format!(
        "RUBASH_NAMEREF_TARGET=value; declare -n RUBASH_NAMEREF_REF=RUBASH_NAMEREF_TARGET; \
         echo read:$RUBASH_NAMEREF_REF > {shell_output_path}; \
         echo bang:${{!RUBASH_NAMEREF_REF}} >> {shell_output_path}; \
         declare -n RUBASH_NAMEREF_REF2=RUBASH_NAMEREF_REF; \
         echo chain:$RUBASH_NAMEREF_REF2 bang2:${{!RUBASH_NAMEREF_REF2}} >> {shell_output_path}; \
         RUBASH_NAMEREF_REF=changed; echo target:$RUBASH_NAMEREF_TARGET >> {shell_output_path}; \
         declare -p RUBASH_NAMEREF_REF >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "read:value\nbang:RUBASH_NAMEREF_TARGET\nchain:value bang2:RUBASH_NAMEREF_TARGET\ntarget:changed\ndeclare -n RUBASH_NAMEREF_REF=\"RUBASH_NAMEREF_TARGET\"\n"
    );
    std::env::remove_var("RUBASH_NAMEREF_TARGET");
    std::env::remove_var("RUBASH_NAMEREF_REF");
    std::env::remove_var("RUBASH_NAMEREF_REF2");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_nameref_unary_conditionals_use_nameref_attribute() {
    let output_path = target_test_path("rubash-nameref-unary-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    for name in [
        "RUBASH_NAMEREF_UNARY_TARGET",
        "RUBASH_NAMEREF_UNARY_REF",
        "RUBASH_NAMEREF_UNARY_PLAIN",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_NAMEREF_UNARY_TARGET=value; \
         RUBASH_NAMEREF_UNARY_PLAIN=value; \
         readonly RUBASH_NAMEREF_UNARY_READONLY=value; \
         declare -n RUBASH_NAMEREF_UNARY_REF=RUBASH_NAMEREF_UNARY_TARGET; \
         [[ -R RUBASH_NAMEREF_UNARY_REF ]]; echo cond_ref:$? > {shell_output_path}; \
         [[ -R RUBASH_NAMEREF_UNARY_PLAIN ]]; echo cond_plain:$? >> {shell_output_path}; \
         [[ -R RUBASH_NAMEREF_UNARY_READONLY ]]; echo cond_readonly:$? >> {shell_output_path}; \
         test -R RUBASH_NAMEREF_UNARY_REF; echo test_ref:$? >> {shell_output_path}; \
         test -R RUBASH_NAMEREF_UNARY_PLAIN; echo test_plain:$? >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "cond_ref:0\ncond_plain:1\ncond_readonly:1\ntest_ref:0\ntest_plain:1\n"
    );
    for name in [
        "RUBASH_NAMEREF_UNARY_TARGET",
        "RUBASH_NAMEREF_UNARY_REF",
        "RUBASH_NAMEREF_UNARY_PLAIN",
        "RUBASH_NAMEREF_UNARY_READONLY",
    ] {
        std::env::remove_var(name);
    }
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_plus_n_clears_nameref_attribute() {
    let output_path = target_test_path("rubash-declare-plus-nameref-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    std::env::remove_var("RUBASH_NAMEREF_CLEAR_TARGET");
    std::env::remove_var("RUBASH_NAMEREF_CLEAR_REF");
    let input = format!(
        "RUBASH_NAMEREF_CLEAR_TARGET=value; declare -n RUBASH_NAMEREF_CLEAR_REF=RUBASH_NAMEREF_CLEAR_TARGET; declare +n RUBASH_NAMEREF_CLEAR_REF; RUBASH_NAMEREF_CLEAR_REF=changed; \
         echo target:$RUBASH_NAMEREF_CLEAR_TARGET ref:$RUBASH_NAMEREF_CLEAR_REF > {shell_output_path}; declare -p RUBASH_NAMEREF_CLEAR_REF >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "target:value ref:changed\ndeclare -- RUBASH_NAMEREF_CLEAR_REF=\"changed\"\n"
    );
    std::env::remove_var("RUBASH_NAMEREF_CLEAR_TARGET");
    std::env::remove_var("RUBASH_NAMEREF_CLEAR_REF");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_nameref_array_elements_read_and_assign_target() {
    let output_path = target_test_path("rubash-nameref-array-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    for name in ["RUBASH_NAMEREF_ARRAY_TARGET", "RUBASH_NAMEREF_ARRAY_REF"] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_NAMEREF_ARRAY_TARGET=(zero one); \
         declare -n RUBASH_NAMEREF_ARRAY_REF=RUBASH_NAMEREF_ARRAY_TARGET; \
         echo read:${{RUBASH_NAMEREF_ARRAY_REF[1]}} len:${{#RUBASH_NAMEREF_ARRAY_REF[@]}} > {shell_output_path}; \
         RUBASH_NAMEREF_ARRAY_REF[1]=ONE; echo assign:${{RUBASH_NAMEREF_ARRAY_TARGET[1]}} >> {shell_output_path}; \
         RUBASH_NAMEREF_ARRAY_REF+=(two); echo append:${{RUBASH_NAMEREF_ARRAY_TARGET[2]}} len:${{#RUBASH_NAMEREF_ARRAY_REF[@]}} >> {shell_output_path}; \
         unset RUBASH_NAMEREF_ARRAY_TARGET[0]; : ${{RUBASH_NAMEREF_ARRAY_REF[0]:=ZERO}}; echo param:${{RUBASH_NAMEREF_ARRAY_TARGET[0]}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "read:one len:2\nassign:ONE\nappend:two len:3\nparam:ZERO\n"
    );
    let _ = fs::remove_file(output_path);
    for name in ["RUBASH_NAMEREF_ARRAY_TARGET", "RUBASH_NAMEREF_ARRAY_REF"] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_nameref_assoc_array_elements_read_keys_and_assign_target() {
    let output_path = target_test_path("rubash-nameref-assoc-array-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    for name in ["RUBASH_NAMEREF_ASSOC_TARGET", "RUBASH_NAMEREF_ASSOC_REF"] {
        std::env::remove_var(name);
    }
    let input = format!(
        "declare -A RUBASH_NAMEREF_ASSOC_TARGET=([k]=v); \
         declare -n RUBASH_NAMEREF_ASSOC_REF=RUBASH_NAMEREF_ASSOC_TARGET; \
         echo read:${{RUBASH_NAMEREF_ASSOC_REF[k]}} len:${{#RUBASH_NAMEREF_ASSOC_REF[@]}} > {shell_output_path}; \
         RUBASH_NAMEREF_ASSOC_REF[k]=V; RUBASH_NAMEREF_ASSOC_REF[new]=N; \
         echo assign:${{RUBASH_NAMEREF_ASSOC_TARGET[k]}} new:${{RUBASH_NAMEREF_ASSOC_TARGET[new]}} len:${{#RUBASH_NAMEREF_ASSOC_REF[@]}} >> {shell_output_path}; \
         printf 'key:%s\\n' ${{!RUBASH_NAMEREF_ASSOC_REF[@]}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "read:v len:1\nassign:V new:N len:2\nkey:k new\n"
    );
    let _ = fs::remove_file(output_path);
    for name in ["RUBASH_NAMEREF_ASSOC_TARGET", "RUBASH_NAMEREF_ASSOC_REF"] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_nameref_parameter_transforms_describe_target() {
    let output_path = target_test_path("rubash-nameref-transform-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    for name in [
        "RUBASH_NAMEREF_TRANSFORM_SCALAR",
        "RUBASH_NAMEREF_TRANSFORM_SCALAR_REF",
        "RUBASH_NAMEREF_TRANSFORM_ASSOC",
        "RUBASH_NAMEREF_TRANSFORM_ASSOC_REF",
        "RUBASH_NAMEREF_TRANSFORM_ARRAY",
        "RUBASH_NAMEREF_TRANSFORM_ARRAY_REF",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_NAMEREF_TRANSFORM_SCALAR=val; \
         declare -n RUBASH_NAMEREF_TRANSFORM_SCALAR_REF=RUBASH_NAMEREF_TRANSFORM_SCALAR; \
         declare -A RUBASH_NAMEREF_TRANSFORM_ASSOC=([k]=v); \
         declare -n RUBASH_NAMEREF_TRANSFORM_ASSOC_REF=RUBASH_NAMEREF_TRANSFORM_ASSOC; \
         RUBASH_NAMEREF_TRANSFORM_ARRAY=(zero one); \
         declare -n RUBASH_NAMEREF_TRANSFORM_ARRAY_REF=RUBASH_NAMEREF_TRANSFORM_ARRAY; \
         echo scalar_attr:${{RUBASH_NAMEREF_TRANSFORM_SCALAR_REF@a}} > {shell_output_path}; \
         echo scalar_decl:${{RUBASH_NAMEREF_TRANSFORM_SCALAR_REF@A}} >> {shell_output_path}; \
         echo assoc_attr:${{RUBASH_NAMEREF_TRANSFORM_ASSOC_REF@a}} >> {shell_output_path}; \
         echo assoc_decl:${{RUBASH_NAMEREF_TRANSFORM_ASSOC_REF@A}} >> {shell_output_path}; \
         echo assoc_k:${{RUBASH_NAMEREF_TRANSFORM_ASSOC_REF@K}} elem:${{RUBASH_NAMEREF_TRANSFORM_ASSOC_REF[k]@K}} >> {shell_output_path}; \
         echo array_k:${{RUBASH_NAMEREF_TRANSFORM_ARRAY_REF@K}} elem:${{RUBASH_NAMEREF_TRANSFORM_ARRAY_REF[0]@K}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "scalar_attr:\nscalar_decl:RUBASH_NAMEREF_TRANSFORM_SCALAR='val'\nassoc_attr:A\nassoc_decl:declare -A RUBASH_NAMEREF_TRANSFORM_ASSOC\nassoc_k: elem:'v'\narray_k:'zero' elem:'zero'\n"
    );
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_NAMEREF_TRANSFORM_SCALAR",
        "RUBASH_NAMEREF_TRANSFORM_SCALAR_REF",
        "RUBASH_NAMEREF_TRANSFORM_ASSOC",
        "RUBASH_NAMEREF_TRANSFORM_ASSOC_REF",
        "RUBASH_NAMEREF_TRANSFORM_ARRAY",
        "RUBASH_NAMEREF_TRANSFORM_ARRAY_REF",
    ] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_local_nameref_restores_after_function() {
    let output_path = target_test_path("rubash-local-nameref-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    std::env::remove_var("RUBASH_LOCAL_NAMEREF_TARGET");
    std::env::remove_var("RUBASH_LOCAL_NAMEREF_REF");
    let input = format!(
        "RUBASH_LOCAL_NAMEREF_TARGET=value; f() {{ local -n RUBASH_LOCAL_NAMEREF_REF=RUBASH_LOCAL_NAMEREF_TARGET; RUBASH_LOCAL_NAMEREF_REF=localchanged; \
                echo in:$RUBASH_LOCAL_NAMEREF_TARGET > {shell_output_path}; }}; \
         f; echo out:$RUBASH_LOCAL_NAMEREF_TARGET >> {shell_output_path}; \
         declare -p RUBASH_LOCAL_NAMEREF_REF 2>/dev/null || echo ref-unset >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:localchanged\nout:localchanged\nref-unset\n"
    );
    std::env::remove_var("RUBASH_LOCAL_NAMEREF_TARGET");
    std::env::remove_var("RUBASH_LOCAL_NAMEREF_REF");
    let _ = fs::remove_file(output_path);
}
