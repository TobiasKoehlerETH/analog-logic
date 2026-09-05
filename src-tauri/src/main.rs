#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod analyzer;
mod dwf;
mod stream;
fn main() {
    tauri::Builder::default()
        .on_window_event(|_, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let _ = stream::stop_blocking();
            }
        })
        .invoke_handler(tauri::generate_handler![
            stream::start_stream,
            stream::stop_stream,
            stream::stream_snapshot,
            stream::decode_stream,
            analyzer::available_decoders,
            analyzer::decoder_details,
            dwf::discover_ad3,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Analog Logic");
}
