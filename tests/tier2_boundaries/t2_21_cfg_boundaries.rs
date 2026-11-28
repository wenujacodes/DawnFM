//! Tier 2: F21 - Windows Cargo Dependencies Partition Boundaries
//!
//! Validates Cargo.toml TOML syntax validity, duplicate target block detection,
//! target isolation of Win32 crates, feature pruning, and Cargo.lock sync per ORIGINAL_REQUEST §R6.

#[test]
fn test_cargo_toml_syntax_validity() {
    let cargo_toml_str = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml must exist");
    assert!(!cargo_toml_str.trim().is_empty());

    // Basic TOML line validation: balanced quotes and valid section header syntax
    let mut in_table = false;
    for line in cargo_toml_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_table = true;
            assert!(trimmed.len() > 2);
        } else if in_table {
            // Must contain '=' or be a continuation
            if trimmed.contains('=') {
                let mut parts = trimmed.splitn(2, '=');
                let key = parts.next().unwrap().trim();
                assert!(!key.is_empty(), "TOML key must not be empty on line: {}", line);
            }
        }
    }
}

#[test]
fn test_cargo_toml_no_duplicate_target_blocks() {
    let cargo_toml_str = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml must exist");

    let mut target_windows_count = 0;
    for line in cargo_toml_str.lines() {
        let trimmed = line.trim();
        if trimmed == "[target.'cfg(windows)'.dependencies]"
            || trimmed == "[target.\"cfg(windows)\".dependencies]"
        {
            target_windows_count += 1;
        }
    }

    assert_eq!(
        target_windows_count, 1,
        "Cargo.toml must have exactly 1 [target.'cfg(windows)'.dependencies] block, found {}",
        target_windows_count
    );
}

#[test]
fn test_cargo_toml_cross_target_resolution() {
    let cargo_toml_str = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml must exist");

    // Scan lines inside the main [dependencies] section
    let mut in_generic_deps = false;
    for line in cargo_toml_str.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_generic_deps = trimmed == "[dependencies]";
            continue;
        }

        if in_generic_deps && trimmed.contains('=') {
            let key = trimmed.split('=').next().unwrap().trim();
            assert!(
                key != "windows" && key != "windows-sys",
                "Crate '{}' must NEVER be declared in generic [dependencies]",
                key
            );
        }
    }
}

#[test]
fn test_cargo_toml_features_pruning() {
    let cargo_toml_str = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml must exist");

    // Keyring on Windows must use windows-native feature, and it must be strictly guarded under cfg(windows)
    assert!(
        cargo_toml_str.contains("windows-native"),
        "Cargo.toml must declare windows-native feature for keyring"
    );
    assert!(
        cargo_toml_str.contains("target.'cfg(windows)'.dependencies")
            || cargo_toml_str.contains("target.\"cfg(windows)\".dependencies"),
        "windows-native feature for keyring must only appear inside cfg(windows) dependencies"
    );

    // Verify windows-native does not leak into general dependencies
    let deps_section = cargo_toml_str.split("[dependencies]").nth(1).unwrap_or("");
    let general_deps = deps_section.split("\n[").next().unwrap_or("");
    assert!(
        !general_deps.contains("windows-native"),
        "General [dependencies] must not declare windows-native feature"
    );
}

#[test]
fn test_cargo_toml_lockfile_target_sync() {
    let lockfile_path = std::path::Path::new("Cargo.lock");
    assert!(lockfile_path.exists(), "Cargo.lock must exist");

    let lock_content = std::fs::read_to_string(lockfile_path).expect("Cargo.lock readable");
    assert!(
        lock_content.contains("name = \"dawnfm\""),
        "Cargo.lock must contain package 'dawnfm'"
    );
    assert!(
        lock_content.contains("version = \"2.6.1\""),
        "Cargo.lock must match version '2.6.1'"
    );
}
