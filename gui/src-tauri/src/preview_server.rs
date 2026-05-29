pub use postkit::mpv::MpvPlayer;

pub fn new_player() -> MpvPlayer {
    MpvPlayer::new("IMFWizard")
}

#[tauri::command]
pub fn preview_set_parent_wid(wid: u64, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.set_parent_wid(wid);
    if state.is_alive() {
        state.kill();
        state.start_mpv()?;
    }
    Ok(())
}

#[tauri::command]
pub fn preview_load(file_path: String, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.load_file(&file_path)
}

#[tauri::command]
pub fn preview_play_pause(state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.play_pause()
}

#[tauri::command]
pub fn preview_seek(seconds: f64, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.seek(seconds)
}

#[tauri::command]
pub fn preview_seek_absolute(
    seconds: f64,
    state: tauri::State<'_, MpvPlayer>,
) -> Result<(), String> {
    state.seek_absolute(seconds)
}

#[tauri::command]
pub fn preview_stop(state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.stop()
}

#[tauri::command]
pub fn preview_get_position(state: tauri::State<'_, MpvPlayer>) -> Result<f64, String> {
    state.get_position()
}

#[tauri::command]
pub fn preview_get_duration(state: tauri::State<'_, MpvPlayer>) -> Result<f64, String> {
    state.get_duration()
}

#[tauri::command]
pub fn preview_get_metadata(state: tauri::State<'_, MpvPlayer>) -> Result<String, String> {
    state.get_metadata()
}

#[tauri::command]
pub fn preview_load_dcp(
    dir_path: String,
    state: tauri::State<'_, MpvPlayer>,
) -> Result<(), String> {
    state.load_package_dir(&dir_path)
}
