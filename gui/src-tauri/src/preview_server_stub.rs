/// Windows preview server using named pipes for mpv IPC.
/// mpv supports `--input-ipc-server=\\.\pipe\name` on Windows.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;

const PIPE_ACCESS_DUPLEX: u32 = 0x00000003;

pub struct MpvPlayer {
    process: Mutex<Option<Child>>,
    pipe_name: String,
}

impl MpvPlayer {
    pub fn new() -> Self {
        let pipe_name = format!(r"\\.\pipe\imfwizard-mpv-{}", std::process::id());
        Self {
            process: Mutex::new(None),
            pipe_name,
        }
    }

    fn is_alive(&self) -> bool {
        let mut proc = self.process.lock().unwrap();
        proc.as_mut().map_or(false, |p| p.try_wait().ok().flatten().is_none())
    }

    fn start_mpv(&self) -> Result<(), String> {
        let mut proc = self.process.lock().unwrap();
        if let Some(mut old) = proc.take() {
            let _ = old.kill();
            let _ = old.wait();
        }

        let args = vec![
            "--idle=yes".to_string(),
            "--no-terminal".to_string(),
            "--keep-open=yes".to_string(),
            "--osc=yes".to_string(),
            format!("--input-ipc-server={}", self.pipe_name),
            "--title=IMFWizard Preview".to_string(),
            "--force-window=yes".to_string(),
        ];

        let child = Command::new("mpv")
            .args(&args)
            .spawn()
            .map_err(|e| format!("Failed to start mpv: {e}"))?;

        *proc = Some(child);

        // Wait for pipe to become connectable
        for _ in 0..50 {
            if self.try_connect().is_ok() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("mpv pipe did not become available".to_string())
    }

    fn ensure_running(&self) -> Result<(), String> {
        if !self.is_alive() {
            self.start_mpv()?;
        }
        Ok(())
    }

    fn try_connect(&self) -> Result<std::fs::File, String> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(PIPE_ACCESS_DUPLEX)
            .open(&self.pipe_name)
            .map_err(|e| format!("Failed to connect to mpv pipe: {e}"))
    }

    fn send_command(&self, cmd: &str) -> Result<String, String> {
        match self.try_send(cmd) {
            Ok(resp) => Ok(resp),
            Err(_) => {
                self.start_mpv()?;
                self.try_send(cmd)
            }
        }
    }

    fn try_send(&self, cmd: &str) -> Result<String, String> {
        let mut pipe = self.try_connect()?;
        pipe.write_all(cmd.as_bytes())
            .map_err(|e| format!("Failed to send: {e}"))?;
        pipe.write_all(b"\n")
            .map_err(|e| format!("Failed to send newline: {e}"))?;

        let mut reader = BufReader::new(pipe);
        let mut response = String::new();
        reader.read_line(&mut response).ok();
        Ok(response)
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.lock().unwrap().take() {
            let _ = child.kill();
        }
    }
}

#[tauri::command]
pub fn preview_load(file_path: String, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {file_path}"));
    }

    state.ensure_running()?;

    let cmd = format!(
        r#"{{"command": ["loadfile", "{}"]}}"#,
        path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
    );
    state.send_command(&cmd)?;
    Ok(())
}

#[tauri::command]
pub fn preview_play_pause(state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.send_command(r#"{"command": ["cycle", "pause"]}"#)?;
    Ok(())
}

#[tauri::command]
pub fn preview_seek(seconds: f64, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.send_command(&format!(r#"{{"command": ["seek", "{seconds}", "relative"]}}"#))?;
    Ok(())
}

#[tauri::command]
pub fn preview_stop(state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.send_command(r#"{"command": ["stop"]}"#)?;
    Ok(())
}

#[tauri::command]
pub fn preview_get_position(state: tauri::State<'_, MpvPlayer>) -> Result<f64, String> {
    let resp = state.send_command(r#"{"command": ["get_property", "time-pos"]}"#)?;
    parse_property_f64(&resp)
}

#[tauri::command]
pub fn preview_get_duration(state: tauri::State<'_, MpvPlayer>) -> Result<f64, String> {
    let resp = state.send_command(r#"{"command": ["get_property", "duration"]}"#)?;
    parse_property_f64(&resp)
}

#[tauri::command]
pub fn preview_seek_absolute(seconds: f64, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    state.send_command(&format!(r#"{{"command": ["seek", "{seconds}", "absolute"]}}"#))?;
    Ok(())
}

#[tauri::command]
pub fn preview_get_metadata(state: tauri::State<'_, MpvPlayer>) -> Result<String, String> {
    let pos = state.send_command(r#"{"command": ["get_property", "time-pos"]}"#).unwrap_or_default();
    let dur = state.send_command(r#"{"command": ["get_property", "duration"]}"#).unwrap_or_default();
    let paused = state.send_command(r#"{"command": ["get_property", "pause"]}"#).unwrap_or_default();
    let fname = state.send_command(r#"{"command": ["get_property", "filename"]}"#).unwrap_or_default();

    Ok(format!(
        r#"{{"position": {}, "duration": {}, "paused": {}, "filename": {}}}"#,
        extract_data_field(&pos),
        extract_data_field(&dur),
        extract_data_field(&paused),
        extract_data_field_str(&fname),
    ))
}

fn parse_property_f64(resp: &str) -> Result<f64, String> {
    if let Some(start) = resp.find("\"data\":") {
        let after = &resp[start + 7..];
        let end = after.find(|c: char| c == ',' || c == '}').unwrap_or(after.len());
        let val_str = after[..end].trim();
        val_str.parse::<f64>().map_err(|e| format!("Parse error: {e} from '{val_str}'"))
    } else {
        Err(format!("No data in response: {resp}"))
    }
}

fn extract_data_field(resp: &str) -> String {
    if let Some(start) = resp.find("\"data\":") {
        let after = &resp[start + 7..];
        let end = after.find(|c: char| c == ',' || c == '}').unwrap_or(after.len());
        after[..end].trim().to_string()
    } else {
        "null".to_string()
    }
}

fn extract_data_field_str(resp: &str) -> String {
    if let Some(start) = resp.find("\"data\":") {
        let after = &resp[start + 7..];
        let end = after.find(|c: char| c == ',' || c == '}').unwrap_or(after.len());
        let val = after[..end].trim();
        if val.starts_with('"') {
            val.to_string()
        } else {
            format!("\"{}\"", val)
        }
    } else {
        "null".to_string()
    }
}

#[tauri::command]
pub fn preview_load_dcp(dir_path: String, state: tauri::State<'_, MpvPlayer>) -> Result<(), String> {
    let dir = PathBuf::from(&dir_path);
    if !dir.is_dir() {
        return Err(format!("Not a directory: {dir_path}"));
    }

    fn find_mxf_files(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    results.extend(find_mxf_files(&path));
                } else if path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("mxf")) {
                    results.push(path);
                }
            }
        }
        results
    }

    let mut mxf_files = find_mxf_files(&dir);
    if mxf_files.is_empty() {
        return Err("No MXF files found in directory".to_string());
    }

    let video_mxf = mxf_files.iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.contains("pic"))
        })
        .cloned()
        .unwrap_or_else(|| {
            mxf_files.sort_by(|a, b| {
                let size_a = a.metadata().map(|m| m.len()).unwrap_or(0);
                let size_b = b.metadata().map(|m| m.len()).unwrap_or(0);
                size_b.cmp(&size_a)
            });
            mxf_files[0].clone()
        });

    state.ensure_running()?;

    let cmd = format!(
        r#"{{"command": ["loadfile", "{}"]}}"#,
        video_mxf.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
    );
    state.send_command(&cmd)?;
    Ok(())
}
