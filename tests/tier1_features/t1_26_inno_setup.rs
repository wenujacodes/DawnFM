//! Tier 1: F26 - Inno Setup Packaging Script
//!
//! Validates Inno Setup installer script syntax, application metadata macros,
//! 64-bit architecture constraints, WebView2 bootstrapper definition, and URL protocol
//! registry integration per ORIGINAL_REQUEST §R6 and BitChord reference script.

const ISS_PATH: &str = "installer/dawnfm.iss";

fn load_iss_script() -> String {
    std::fs::read_to_string(ISS_PATH)
        .unwrap_or_else(|e| panic!("Genuine installer script must exist at '{}': {}", ISS_PATH, e))
}

#[test]
fn test_iss_script_file_exists() {
    let path = std::path::Path::new(ISS_PATH);
    assert!(
        path.exists(),
        "Genuine installer script must exist in repository at '{}'",
        ISS_PATH
    );
    let content = load_iss_script();
    assert!(
        !content.trim().is_empty(),
        "installer/dawnfm.iss must not be empty"
    );
    assert!(
        content.contains("[Setup]"),
        "installer/dawnfm.iss must declare [Setup] section"
    );
}

#[test]
fn test_iss_script_app_metadata() {
    let script = load_iss_script();

    assert!(
        script.contains("#define MyAppName \"Dawn FM\""),
        "Script must define MyAppName macro as 'Dawn FM'"
    );
    assert!(
        script.contains("#define MyAppVersion \"2.6.1\""),
        "Script must define MyAppVersion matching crate version"
    );
    assert!(
        script.contains("#define MyAppExeName \"dawnfm.exe\""),
        "Script must define MyAppExeName as 'dawnfm.exe'"
    );
}

#[test]
fn test_iss_script_architecture_64bit() {
    let script = load_iss_script();

    assert!(
        script.contains("ArchitecturesAllowed=x64compatible") || script.contains("ArchitecturesAllowed={#Arch}"),
        "Script must enforce x64compatible architecture"
    );
    assert!(
        script.contains("ArchitecturesInstallIn64BitMode=x64compatible") || script.contains("ArchitecturesInstallIn64BitMode={#Arch}"),
        "Script must install in 64-bit mode on x64 systems"
    );
}

#[test]
fn test_iss_script_webview2_bootstrapper() {
    let script = load_iss_script();

    assert!(
        script.contains("[Code]"),
        "Script must contain [Code] section for bootstrapper Pascal script"
    );
    assert!(
        script.contains("NeedsWebView2"),
        "Script must declare NeedsWebView2 runtime detection check"
    );
}

#[test]
fn test_iss_script_protocol_registration() {
    let script = load_iss_script();

    assert!(
        script.contains("[Registry]"),
        "Script must contain [Registry] section for Windows registry keys"
    );
    assert!(
        script.contains("URL Protocol"),
        "Script must register URL Protocol for deep linking"
    );
    assert!(
        script.contains(r#"shell\open\command"#),
        "Script must register shell open command for deeplink protocol"
    );
}
