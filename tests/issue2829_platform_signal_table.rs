//! rubash#2829: runtime `Platform` semantic axis — first slice (signal table).
//!
//! Verifies that `Platform::{Linux, MacOs, Windows, Android, FreeBsd}` all
//! share one unified signal table and can be instantiated and tested on ANY
//! host without `#[cfg]` gates on the semantic assertions.

use rubash::builtins::{kill, trap};
use rubash::lexer::tokenize;
use rubash::parser::parse;
use rubash::platform::{signal_name_for_number, signal_number_for_name, Platform};
use rubash::shell::var_table::VarTable;
use rubash::Executor;

const ALL_PLATFORMS: [Platform; 5] = [
    Platform::Linux,
    Platform::Android,
    Platform::MacOs,
    Platform::FreeBsd,
    Platform::Windows,
];

#[test]
fn all_platform_tables_satisfy_structural_invariants() {
    for platform in ALL_PLATFORMS {
        let table = platform.signal_table();
        assert!(!table.is_empty(), "{platform:?} signal table is empty");
        for (idx, entry) in table.iter().enumerate() {
            assert_eq!(entry.number, (idx + 1) as i32, "{platform:?} slot {idx}");
            assert_eq!(
                entry.number_str.parse::<i32>().ok(),
                Some(entry.number),
                "{platform:?} number_str {:?}",
                entry.number_str
            );
            if entry.sig_name.is_empty() {
                assert!(
                    entry.bare_name.is_empty(),
                    "{platform:?} hole {} has bare_name {:?}",
                    entry.number,
                    entry.bare_name
                );
                assert_eq!(signal_name_for_number(platform, entry.number), None);
            } else {
                assert_eq!(
                    entry.sig_name,
                    format!("SIG{}", entry.bare_name),
                    "{platform:?} slot {}",
                    entry.number
                );
                assert_eq!(
                    signal_name_for_number(platform, entry.number),
                    Some(entry.bare_name)
                );
                assert_eq!(
                    signal_number_for_name(platform, entry.bare_name),
                    Some(entry.number)
                );
                assert_eq!(
                    signal_number_for_name(platform, entry.sig_name),
                    Some(entry.number)
                );
                assert_eq!(
                    signal_number_for_name(platform, &entry.sig_name.to_ascii_lowercase()),
                    Some(entry.number)
                );
            }
        }
    }
}

#[test]
fn host_platform_matches_compiled_target() {
    #[cfg(target_os = "linux")]
    assert_eq!(Platform::host(), Platform::Linux);
    #[cfg(target_os = "android")]
    assert_eq!(Platform::host(), Platform::Android);
    #[cfg(target_os = "macos")]
    assert_eq!(Platform::host(), Platform::MacOs);
    #[cfg(target_os = "freebsd")]
    assert_eq!(Platform::host(), Platform::FreeBsd);
    #[cfg(not(unix))]
    assert_eq!(Platform::host(), Platform::Windows);
}

#[test]
fn signal_table_matrix_across_linux_macos_and_windows() {
    // Linux and Windows wire-format share Linux numbering (64 slots, holes 32..=33).
    for platform in [Platform::Linux, Platform::Windows] {
        assert_eq!(platform.signal_table().len(), 64, "{platform:?}");
        assert_eq!(signal_number_for_name(platform, "USR1"), Some(10));
        assert_eq!(signal_number_for_name(platform, "SIGUSR2"), Some(12));
        assert_eq!(signal_number_for_name(platform, "STKFLT"), Some(16));
        assert_eq!(signal_number_for_name(platform, "CHLD"), Some(17));
        assert_eq!(signal_number_for_name(platform, "CONT"), Some(18));
        assert_eq!(signal_number_for_name(platform, "STOP"), Some(19));
        assert_eq!(signal_number_for_name(platform, "TSTP"), Some(20));
        assert_eq!(signal_number_for_name(platform, "PWR"), Some(30));
        assert_eq!(signal_number_for_name(platform, "RTMIN"), Some(34));
        assert_eq!(signal_number_for_name(platform, "RTMAX"), Some(64));
        assert_eq!(signal_name_for_number(platform, 7), Some("BUS"));
        assert_eq!(signal_name_for_number(platform, 32), None);
        assert_eq!(signal_name_for_number(platform, 33), None);
        assert_eq!(signal_name_for_number(platform, 34), Some("RTMIN"));
        assert_eq!(signal_name_for_number(platform, 64), Some("RTMAX"));
        assert_eq!(signal_name_for_number(platform, 65), None);
    }

    // macOS / FreeBSD (BSD family): 31 slots, no RT signals, reordered slots.
    for platform in [Platform::MacOs, Platform::FreeBsd] {
        assert_eq!(platform.signal_table().len(), 31, "{platform:?}");
        assert_eq!(signal_number_for_name(platform, "EMT"), Some(7));
        assert_eq!(signal_number_for_name(platform, "BUS"), Some(10));
        assert_eq!(signal_number_for_name(platform, "SYS"), Some(12));
        assert_eq!(signal_number_for_name(platform, "URG"), Some(16));
        assert_eq!(signal_number_for_name(platform, "STOP"), Some(17));
        assert_eq!(signal_number_for_name(platform, "TSTP"), Some(18));
        assert_eq!(signal_number_for_name(platform, "CONT"), Some(19));
        assert_eq!(signal_number_for_name(platform, "CHLD"), Some(20));
        assert_eq!(signal_number_for_name(platform, "INFO"), Some(29));
        assert_eq!(signal_number_for_name(platform, "USR1"), Some(30));
        assert_eq!(signal_number_for_name(platform, "SIGUSR2"), Some(31));
        assert_eq!(signal_number_for_name(platform, "STKFLT"), None);
        assert_eq!(signal_number_for_name(platform, "PWR"), None);
        assert_eq!(signal_number_for_name(platform, "RTMIN"), None);
        assert_eq!(signal_number_for_name(platform, "RTMAX"), None);
        assert_eq!(signal_name_for_number(platform, 17), Some("STOP"));
        assert_eq!(signal_name_for_number(platform, 20), Some("CHLD"));
        assert_eq!(signal_name_for_number(platform, 32), None);
        assert_eq!(signal_name_for_number(platform, 64), None);
    }

    // Android (bionic): 64 slots, holes 32..=40, RT block at 41..=64.
    assert_eq!(Platform::Android.signal_table().len(), 64);
    assert_eq!(signal_name_for_number(Platform::Android, 32), None);
    assert_eq!(signal_name_for_number(Platform::Android, 40), None);
    assert_eq!(signal_name_for_number(Platform::Android, 41), Some("RTMIN"));
    assert_eq!(
        signal_name_for_number(Platform::Android, 52),
        Some("RTMIN+11")
    );
    assert_eq!(
        signal_name_for_number(Platform::Android, 53),
        Some("RTMAX-11")
    );
    assert_eq!(signal_name_for_number(Platform::Android, 64), Some("RTMAX"));
}

#[test]
fn kill_and_trap_builtins_follow_runtime_platform_on_any_host() {
    for (platform, expected_usr1, expected_17, has_rt) in [
        (Platform::Linux, "10\n", "CHLD\n", true),
        (Platform::Windows, "10\n", "CHLD\n", true),
        (Platform::MacOs, "30\n", "STOP\n", false),
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let rc = kill::execute_with_io_posix_for_platform(
            &["-l".into(), "USR1".into()],
            &mut out,
            &mut err,
            false,
            platform,
        )
        .unwrap();
        assert_eq!(rc, 0, "{platform:?} kill -l USR1");
        assert_eq!(String::from_utf8_lossy(&out), expected_usr1);

        out.clear();
        err.clear();
        let rc = kill::execute_with_io_posix_for_platform(
            &["-l".into(), "17".into()],
            &mut out,
            &mut err,
            false,
            platform,
        )
        .unwrap();
        assert_eq!(rc, 0, "{platform:?} kill -l 17");
        assert_eq!(String::from_utf8_lossy(&out), expected_17);

        out.clear();
        err.clear();
        let rc = kill::execute_with_io_posix_for_platform(
            &["-l".into(), "RTMIN".into()],
            &mut out,
            &mut err,
            false,
            platform,
        )
        .unwrap();
        if has_rt {
            assert_eq!(rc, 0);
            assert_eq!(String::from_utf8_lossy(&out), "34\n");
        } else {
            assert_eq!(rc, 1);
            assert!(out.is_empty());
        }

        let mut env_vars = VarTable::default();
        out.clear();
        err.clear();
        let rc = trap::execute_with_io_for_platform(
            &["-l".into()],
            &mut env_vars,
            &mut out,
            &mut err,
            platform,
        )
        .unwrap();
        assert_eq!(rc, 0, "{platform:?} trap -l");
        let listing = String::from_utf8_lossy(&out);
        if has_rt {
            assert!(listing.contains("SIGRTMAX"), "{platform:?}: {listing}");
            assert!(listing.contains("SIGSTKFLT"), "{platform:?}: {listing}");
        } else {
            assert!(listing.contains("SIGEMT"), "{platform:?}: {listing}");
            assert!(listing.contains("SIGINFO"), "{platform:?}: {listing}");
            assert!(!listing.contains("SIGRTMIN"), "{platform:?}: {listing}");
        }
    }
}

#[test]
fn executor_platform_axis_controls_signal_builtins() {
    let mut executor = Executor::new();
    assert_eq!(executor.platform(), Platform::host());

    let ast = parse(&tokenize("OUT=$(kill -l USR1)"));

    executor.set_platform(Platform::MacOs);
    assert_eq!(executor.platform(), Platform::MacOs);
    executor.execute_ast(&ast).expect("macos kill -l USR1");
    assert_eq!(executor.get_env("OUT"), Some("30"));

    executor.set_platform(Platform::Linux);
    assert_eq!(executor.platform(), Platform::Linux);
    executor.execute_ast(&ast).expect("linux kill -l USR1");
    assert_eq!(executor.get_env("OUT"), Some("10"));

    executor.set_platform(Platform::Windows);
    assert_eq!(executor.platform(), Platform::Windows);
    executor.execute_ast(&ast).expect("windows kill -l USR1");
    assert_eq!(executor.get_env("OUT"), Some("10"));
}
