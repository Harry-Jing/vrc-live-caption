use crate::error::{AppError, AppResult};
use std::fs;
use std::path::Path;

fn production_lib_source() -> &'static str {
    include_str!("lib.rs")
        .split("#[path = \"lib_tests.rs\"]")
        .next()
        .unwrap_or_default()
}

#[test]
fn crate_root_delegates_desktop_wiring_to_the_desktop_facade() {
    let production_lib = production_lib_source();

    assert!(production_lib.contains("mod desktop;"));
    assert!(production_lib.contains("desktop::install"));
    assert!(production_lib.contains(".run(desktop::handle_run_event)"));

    for leaked_implementation in ["mod commands;", "mod state;", "commands::", "AppState"] {
        assert!(
            !production_lib.contains(leaked_implementation),
            "crate root leaked desktop implementation detail: {leaked_implementation}"
        );
    }
}

fn read_json_version(path: &Path) -> AppResult<String> {
    let contents = fs::read_to_string(path).map_err(|error| {
        AppError::config_io(format!("Failed to read {}: {error}", path.display()))
    })?;
    let document = serde_json::from_str::<serde_json::Value>(&contents).map_err(|error| {
        AppError::config(format!("Failed to parse {}: {error}", path.display()))
    })?;

    document
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::config(format!(
                "{} must define version as a string",
                path.display()
            ))
        })
}

#[test]
fn repository_manifests_match_cargo_package_version() -> AppResult<()> {
    let cargo_manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = cargo_manifest_dir
        .parent()
        .ok_or_else(|| AppError::config("Cargo manifest directory must have a repository root"))?;
    let cargo_package_version = env!("CARGO_PKG_VERSION");

    let root_package_version = read_json_version(&repo_root.join("package.json"))?;
    let tauri_config_version = read_json_version(&cargo_manifest_dir.join("tauri.conf.json"))?;

    assert_eq!(
        root_package_version, cargo_package_version,
        "package.json version must match the Cargo package version"
    );
    assert_eq!(
        tauri_config_version, cargo_package_version,
        "tauri.conf.json version must match the Cargo package version"
    );

    Ok(())
}

#[test]
fn unsaved_changes_dialog_has_a_narrow_message_permission() {
    let production_lib = production_lib_source();
    let desktop_capability = include_str!("../capabilities/default.json");

    assert!(production_lib.contains("tauri_plugin_dialog::init()"));
    assert!(desktop_capability.contains("\"dialog:allow-message\""));

    for permission in ["dialog:default", "dialog:allow-open", "dialog:allow-save"] {
        assert!(!desktop_capability.contains(&format!("\"{permission}\"")));
    }
}

#[test]
fn diagnostic_report_has_write_only_clipboard_permissions() {
    let production_lib = production_lib_source();
    let desktop_capability = include_str!("../capabilities/default.json");

    assert!(production_lib.contains("tauri_plugin_clipboard_manager::init()"));
    assert!(desktop_capability.contains("\"core:app:allow-version\""));
    assert!(desktop_capability.contains("\"clipboard-manager:allow-write-text\""));

    for permission in [
        "clipboard-manager:default",
        "clipboard-manager:allow-read-text",
        "clipboard-manager:allow-read-image",
        "clipboard-manager:allow-write-image",
        "clipboard-manager:allow-clear",
    ] {
        assert!(!desktop_capability.contains(&format!("\"{permission}\"")));
    }
}
// Temporary #41 acceptance probe; removed before this PR is ready.
#[test]
fn ci_stuck_test_probe() {
    if std::env::var_os("VRC_CI_GATE_PROBE").is_some() {
        loop {
            std::thread::park();
        }
    }
}
