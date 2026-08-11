// Keep this ACL manifest in sync with the invoke handler in src/desktop.rs.
const APP_COMMANDS: &[&str] = &[
    "save_app_config",
    "list_audio_input_devices",
    "probe_audio_input",
    "start_runtime",
    "stop_runtime",
    "get_runtime_control_snapshot",
    "get_caption_aggregate_snapshot",
    "send_osc_test_message",
    "save_credential",
    "delete_credential",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // tauri-build embeds Common Controls v6 only in app binaries. Declaring
        // it at link time also gives Windows test executables a v6 manifest.
        // Remove this workaround after https://github.com/tauri-apps/tauri/issues/13419 is fixed.
        println!(
            "cargo::rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )?;

    Ok(())
}
