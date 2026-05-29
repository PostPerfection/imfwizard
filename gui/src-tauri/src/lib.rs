#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(unused_imports)]
use tauri::Manager;

mod pipeline;
#[cfg(unix)]
mod preview_server;
#[cfg(not(unix))]
mod preview_server_stub;
#[cfg(not(unix))]
use preview_server_stub as preview_server;

/// Fork a parent process that waits for the app to exit, then unconditionally
/// restores terminal settings. This handles WebKitGTK child processes that
/// corrupt the terminal after the main process exits.
#[cfg(unix)]
fn fork_terminal_guard() {
    unsafe {
        // Check if we have a terminal
        if libc::isatty(libc::STDIN_FILENO) == 0 {
            return;
        }

        let mut saved: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut saved);

        let pid = libc::fork();
        if pid < 0 {
            return; // Fork failed, proceed without guard
        }
        if pid > 0 {
            // Parent: wait for child to exit, then restore terminal
            let mut status: libc::c_int = 0;
            libc::waitpid(pid, &mut status, 0);
            // Wait for orphaned WebKitGTK processes to settle
            libc::usleep(100_000); // 100ms
                                   // Force terminal back to sane state
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &saved);
            // Also explicitly reset via stty as a last resort
            libc::system(b"stty sane 2>/dev/null\0".as_ptr() as *const _);
            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            };
            std::process::exit(exit_code);
        }
        // Child continues to run the app
        // Redirect stdin so WebKitGTK subprocesses can't touch the terminal
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY);
        if devnull >= 0 {
            libc::dup2(devnull, libc::STDIN_FILENO);
            libc::close(devnull);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(unix)]
    fork_terminal_guard();

    let mpv = preview_server::MpvPlayer::new();
    let job_queue = pipeline::JobQueue::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_fs::init())
        .manage(mpv)
        .manage(job_queue)
        .invoke_handler(tauri::generate_handler![
            preview_server::preview_load,
            preview_server::preview_play_pause,
            preview_server::preview_seek,
            preview_server::preview_seek_absolute,
            preview_server::preview_stop,
            preview_server::preview_load_dcp,
            preview_server::preview_get_position,
            preview_server::preview_get_duration,
            preview_server::preview_get_metadata,
            preview_server::preview_set_parent_wid,
            pipeline::submit_job,
            pipeline::cancel_job,
            pipeline::pause_job,
            pipeline::resume_job,
            pipeline::list_jobs,
        ])
        .setup(|_app| Ok(()))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(mpv) = window.try_state::<preview_server::MpvPlayer>() {
                    mpv.kill();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
