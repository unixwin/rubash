use super::names::valid_declare_name;

#[test]
fn invalid_declare_names_are_rejected_before_assignment() {
    assert!(!valid_declare_name("[]=asdf"));
    assert!(!valid_declare_name("a[]=asdf"));
    assert!(!valid_declare_name("=asdf"));
    assert!(valid_declare_name("BASH_ARGV[1]=foo"));
    assert!(valid_declare_name("name=value"));
    assert!(valid_declare_name("name+=value"));
}
