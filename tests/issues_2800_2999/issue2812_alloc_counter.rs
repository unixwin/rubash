//! Tests for rubash#2812: feature-gated allocation counting (`--features alloc-counter`).

use std::process::Command;

#[cfg(not(feature = "alloc-counter"))]
#[test]
fn default_build_has_no_alloc_stats_output() {
    let bin = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(bin)
        .env("RUBASH_ALLOC_STATS", "1")
        .args(["-c", "x=hello; echo $x"])
        .output()
        .expect("run rubash binary");
    assert!(output.status.success(), "rubash failed: {:?}", output);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("alloc_count="),
        "default build must not emit alloc stats, got stderr: {stderr}"
    );
}

#[cfg(feature = "alloc-counter")]
mod alloc_counter_enabled {
    use super::*;
    use std::collections::BTreeMap;

    fn parse_stats_line(text: &str) -> BTreeMap<&str, u64> {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            if line.contains("alloc_count=") {
                for tok in line.split_whitespace() {
                    if let Some((k, v)) = tok.split_once('=') {
                        if let Ok(n) = v.parse::<u64>() {
                            map.insert(k, n);
                        }
                    }
                }
            }
        }
        map
    }

    #[test]
    fn library_api_tracks_and_resets_allocations() {
        rubash::reset_alloc_stats();
        let before = rubash::alloc_stats();
        assert_eq!(before.alloc_count, 0);
        assert_eq!(before.alloc_bytes, 0);

        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        buf.push(1);
        buf.resize(8192, 2);
        let mid = rubash::alloc_stats();
        assert!(
            mid.alloc_count >= 1,
            "expected alloc_count >= 1, got {mid:?}"
        );
        assert!(
            mid.alloc_bytes >= 4096,
            "expected alloc_bytes >= 4096, got {mid:?}"
        );
        assert!(
            mid.peak_bytes >= 8192,
            "expected peak_bytes >= 8192, got {mid:?}"
        );
        drop(buf);

        let after = rubash::alloc_stats();
        assert!(
            after.dealloc_count >= 1,
            "expected dealloc_count >= 1, got {after:?}"
        );
    }

    #[test]
    fn binary_emits_deterministic_stats_to_stderr_and_file() {
        let bin = env!("CARGO_BIN_EXE_rubash");
        let script = "x=0; for i in 1 2 3 4 5; do x=$((x + i)); done; echo $x";

        let run1 = Command::new(bin)
            .env("RUBASH_ALLOC_STATS", "1")
            .args(["-c", script])
            .output()
            .expect("run1");
        assert!(run1.status.success(), "run1 failed: {:?}", run1);
        assert_eq!(String::from_utf8_lossy(&run1.stdout).trim(), "15");
        let stderr1 = String::from_utf8_lossy(&run1.stderr);
        let stats1 = parse_stats_line(&stderr1);

        let run2 = Command::new(bin)
            .env("RUBASH_ALLOC_STATS", "1")
            .args(["-c", script])
            .output()
            .expect("run2");
        assert!(run2.status.success(), "run2 failed: {:?}", run2);
        assert_eq!(String::from_utf8_lossy(&run2.stdout).trim(), "15");
        let stderr2 = String::from_utf8_lossy(&run2.stderr);
        let stats2 = parse_stats_line(&stderr2);

        for key in [
            "alloc_count",
            "dealloc_count",
            "realloc_count",
            "alloc_bytes",
            "live_bytes",
            "peak_bytes",
        ] {
            let v1 = stats1
                .get(key)
                .copied()
                .unwrap_or_else(|| panic!("missing {key} in run1 stderr: {stderr1}"));
            let v2 = stats2
                .get(key)
                .copied()
                .unwrap_or_else(|| panic!("missing {key} in run2 stderr: {stderr2}"));
            let max_delta = (v1.max(v2) / 50).max(16);
            assert!(
                v1.abs_diff(v2) <= max_delta,
                "divergent {key}: {v1} vs {v2} (allowed delta {max_delta})"
            );
        }
        assert!(stats1["alloc_count"] > 0);
        assert!(stats1["alloc_bytes"] > 0);
        assert!(stats1["peak_bytes"] > 0);

        let temp_path =
            std::env::temp_dir().join(format!("rubash_2812_stats_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&temp_path);

        let run_file = Command::new(bin)
            .env("RUBASH_ALLOC_STATS", &temp_path)
            .args(["-c", script])
            .output()
            .expect("run_file");
        assert!(run_file.status.success(), "run_file failed: {:?}", run_file);
        assert!(
            !String::from_utf8_lossy(&run_file.stderr).contains("alloc_count="),
            "file mode should not write stats to stderr"
        );
        let file_contents = std::fs::read_to_string(&temp_path).expect("read stats file");
        let _ = std::fs::remove_file(&temp_path);
        let file_stats = parse_stats_line(&file_contents);
        assert!(file_stats["alloc_count"] > 0);
        assert!(file_stats["alloc_bytes"] > 0);
        assert!(file_stats["peak_bytes"] > 0);
        let max_count_delta = (stats1["alloc_count"] / 50).max(16);
        assert!(file_stats["alloc_count"].abs_diff(stats1["alloc_count"]) <= max_count_delta);
    }
}
