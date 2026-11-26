//! rubash#1393 — a multi-element `"${a[@]}"` used as a case PATTERN was
//! space-joined into one pattern; GNU uses the FIRST element as the
//! pattern (the rest are discarded), so `x` matches but `x y` does not.
//!
//! Probe: clusters/k9/probes/p1393.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn case_pattern_from_array_uses_first_element() {
    let (stdout, stderr, code) = rubash(
        "a=(x y); case x in \"${a[@]}\") echo ALL ;; *) echo NO ;; esac\n\
         case \"x y\" in \"${a[@]}\") echo ALL2 ;; *) echo NO2 ;; esac\n\
         a=(\"x*\" y); case \"x*\" in \"${a[@]}\") echo ALL3 ;; *) echo NO3 ;; esac\n\
         a=(x y); case y in \"${a[@]}\") echo M-y ;; *) echo N-y ;; esac\n\
         a=(x y); case \"pre x\" in \"pre ${a[@]}\") echo M1 ;; *) echo N1 ;; esac\n\
         a=(x y); case \"x post\" in \"${a[@]} post\") echo M2 ;; *) echo N2 ;; esac\n\
         a=(only); case \"only post\" in \"${a[@]} post\") echo M5 ;; *) echo N5 ;; esac",
    );
    assert_eq!(stdout, "ALL\nNO2\nALL3\nN-y\nM1\nN2\nM5\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
