mod audio;
mod caption;
mod caption_pipeline;
mod chatbox;
mod config;
mod credentials;
mod desktop;
mod error;
mod events;
mod generation_fence;
mod host_resolver;
mod recognition;
mod runtime;
mod runtime_control;
mod saved_settings;
mod system_proxy;
mod translation;
mod wall_clock;

#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable and should include the canonical startup context."
)]
pub fn run() {
    let _ = tracing_subscriber::fmt().with_target(false).try_init();

    desktop::install(
        tauri::Builder::default()
            .plugin(tauri_plugin_clipboard_manager::init())
            .plugin(tauri_plugin_dialog::init()),
    )
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(desktop::handle_run_event);
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
