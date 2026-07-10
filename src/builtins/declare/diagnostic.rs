use std::env;

pub(super) fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        env::var("__RUBASH_SCRIPT_NAME"),
        env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }

    "rubash: ".to_string()
}
