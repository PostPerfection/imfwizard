#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

const PROJECT_FILE_EXTENSION: &str = "imfwizard";
const MAIN_WINDOW_LABEL: &str = "main";
#[cfg(target_os = "linux")]
const MAIN_WEBVIEW_LABEL: &str = "main-webview";
#[cfg(target_os = "linux")]
const MAIN_WINDOW_TITLE: &str = "IMF Wizard — IMP Creator";
#[cfg(target_os = "linux")]
const MAIN_WINDOW_WIDTH: f64 = 900.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_HEIGHT: f64 = 700.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_MINIMUM_WIDTH: f64 = 700.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_MINIMUM_HEIGHT: f64 = 500.0;
#[cfg(target_os = "linux")]
const MAIN_WINDOW_BACKGROUND: tauri::window::Color = tauri::window::Color(0, 0, 0, 255);

mod pipeline;
mod preferences;
mod timeline_cmd;

#[tauri::command]
fn component_versions(
    preview_player: tauri::State<'_, guikit::preview::PreviewPlayer>,
) -> Vec<postkit::component_versions::ComponentVersion> {
    guikit::component_versions::installed_components(
        "IMF Wizard",
        env!("CARGO_PKG_VERSION"),
        &preview_player,
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    guikit_startup::prefer_shared_memory_webkit_frames_on_nvidia();
    postkit::grok_encoder::set_packaged_gpu_plugin_path("imfwizard");

    #[cfg(unix)]
    guikit_startup::fork_terminal_guard();

    let job_queue = pipeline::JobQueue::new(pipeline::jobs_path());
    job_queue.load_jobs_file();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .manage(job_queue)
        .manage(guikit::launch_project::LaunchProject::default())
        .invoke_handler(tauri::generate_handler![
            guikit::preview::preview_load,
            guikit::preview::preview_play_pause,
            guikit::preview::preview_seek,
            guikit::preview::preview_seek_absolute,
            guikit::preview::preview_frame_step,
            guikit::preview::preview_frame_back_step,
            guikit::preview::preview_stop,
            guikit::preview::preview_load_dcp,
            guikit::preview::preview_get_position,
            guikit::preview::preview_get_duration,
            guikit::preview::preview_get_metadata,
            guikit::preview::preview_set_surface,
            guikit::preview::preview_is_embedded,
            guikit::preview::preview_set_overlays,
            guikit::preview::preview_set_decode_scale,
            guikit::preview::preview_set_subtitle_file,
            guikit::preview::preview_set_subtitle_visibility,
            guikit::preview::preview_set_picture_filters,
            guikit::preview::preview_takes_picture_filters,
            guikit::preview::player_controls::preview_set_picture,
            guikit::preview::player_controls::preview_set_sound_device,
            guikit::preview::player_controls::preview_set_sound_layout,
            guikit::preview::player_controls::preview_set_sound_delay,
            guikit::preview::player_controls::preview_set_subtitle_presentation,
            guikit::preview::player_controls::preview_sound_devices,
            guikit::preview::player_controls::preview_set_display_profile,
            guikit::preview::player_controls::preview_set_stereo_output,
            guikit::preview::player_controls::preview_set_level_meter,
            guikit::preview::player_controls::preview_loaded_picture,
            guikit::gpu::set_gpu,
            component_versions,
            pipeline::validate_imp,
            guikit::launch_project::take_launch_project_path,
            preferences::load_preferences,
            preferences::save_preferences,
            preferences::reset_preferences,
            pipeline::submit_job,
            pipeline::cancel_job,
            pipeline::move_job,
            pipeline::pause_job,
            pipeline::resume_job,
            pipeline::list_jobs,
            pipeline::retitle_imp,
            pipeline::delete_imp,
            pipeline::disk_space,
            pipeline::detect_source_crop,
            pipeline::subtitle_file_for_preview,
            pipeline::audio_map_shape,
            timeline_cmd::list_cpls,
            timeline_cmd::get_timeline,
        ])
        .setup(|app| {
            #[cfg(target_os = "linux")]
            guikit::startup::create_main_window(
                app,
                &guikit::startup::MainWindow {
                    label: MAIN_WINDOW_LABEL,
                    webview_label: MAIN_WEBVIEW_LABEL,
                    title: MAIN_WINDOW_TITLE,
                    width: MAIN_WINDOW_WIDTH,
                    height: MAIN_WINDOW_HEIGHT,
                    minimum_width: MAIN_WINDOW_MINIMUM_WIDTH,
                    minimum_height: MAIN_WINDOW_MINIMUM_HEIGHT,
                    background: MAIN_WINDOW_BACKGROUND,
                },
            )?;
            app.manage(guikit::preview::create_player(app, MAIN_WINDOW_LABEL));
            guikit::launch_project::store_from_args(app.handle(), PROJECT_FILE_EXTENSION);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            guikit::launch_project::handle_run_event(app, &event, PROJECT_FILE_EXTENSION);
            if let tauri::RunEvent::ExitRequested { .. } = event {
                app.state::<guikit::preview::PreviewPlayer>().shutdown();
                app.state::<pipeline::JobQueue>().stop_for_exit();
            }
        });
}
