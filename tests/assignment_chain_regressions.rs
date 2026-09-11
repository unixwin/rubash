//! GNU bash binds prefix assignment words left-to-right, each expansion
//! seeing the previous binding (busybox ash-vars/var4 differential; GNU
//! bash 5.2.21 script-file baseline). Rubash stored assignments in a
//! HashMap (arbitrary order) and expanded pure-assignment words up front,
//! so chained assignments like X=usbdev1.2 X=${X#usbdev} saw nothing.

use std::process::Command;

fn rubash(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn pure_assignment_line_chains() {
    assert_eq!(
        rubash("X=usbdev1.2 X=${X#usbdev} B=${X%%.*} D=${X#*.}\necho bus/usb/$B/$D"),
        "bus/usb/1/2\n"
    );
}

#[test]
fn temporary_assignment_env_chains() {
    assert_eq!(rubash("a=1 b=$a env | grep '^b='"), "b=1\n");
}

#[test]
fn temporary_assignment_env_does_not_leak() {
    assert_eq!(rubash("a=1 b=$a true; echo after:$b"), "after:\n");
}

#[test]
fn semicolon_command_sees_chained_values() {
    assert_eq!(
        rubash("X=usbdev1.2 X=${X#usbdev} B=${X%%.*} D=${X#*.}; echo bus/usb/$B/$D"),
        "bus/usb/1/2\n"
    );
}
