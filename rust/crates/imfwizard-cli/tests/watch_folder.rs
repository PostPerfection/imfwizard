use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const POLL_INTERVAL_SECONDS: &str = "1";
const BUILD_TIMEOUT: Duration = Duration::from_secs(180);
const FAILURE_TIMEOUT: Duration = Duration::from_secs(60);
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_millis(100);

struct RecordedWebhooks {
    url: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl RecordedWebhooks {
    fn wait_for_one(&self, event_type: &str) -> Vec<String> {
        let deadline = Instant::now() + WEBHOOK_TIMEOUT;
        loop {
            let bodies = self.bodies.lock().unwrap().clone();
            let matching: Vec<String> = bodies
                .into_iter()
                .filter(|body| body.contains(event_type))
                .collect();
            if !matching.is_empty() || Instant::now() >= deadline {
                return matching;
            }
            std::thread::sleep(CHECK_INTERVAL);
        }
    }
}

fn answer_request(stream: TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        let lowercase = line.to_ascii_lowercase();
        if let Some(value) = lowercase.strip_prefix("content-length:") {
            content_length = value.trim().parse().ok()?;
        }
    }

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;
    let mut stream = reader.into_inner();
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .ok()?;
    stream.flush().ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn start_webhook_listener() -> RecordedWebhooks {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&bodies);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            if let Some(body) = answer_request(stream) {
                recorded.lock().unwrap().push(body);
            }
        }
    });

    RecordedWebhooks { url, bodies }
}

struct Watcher {
    child: Child,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_watcher(
    watch_dir: &Path,
    output_dir: &Path,
    webhooks: &RecordedWebhooks,
    config_home: &Path,
) -> Watcher {
    start_watcher_with(
        watch_dir,
        output_dir,
        webhooks,
        config_home,
        POLL_INTERVAL_SECONDS,
        &[],
    )
}

fn start_watcher_with(
    watch_dir: &Path,
    output_dir: &Path,
    webhooks: &RecordedWebhooks,
    config_home: &Path,
    interval_seconds: &str,
    create_arguments: &[&str],
) -> Watcher {
    let mut command = Command::new(env!("CARGO_BIN_EXE_imfwizard"));
    command
        .env("XDG_CONFIG_HOME", config_home)
        .args([
            "watch",
            watch_dir.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--interval",
            interval_seconds,
            "--webhook-url",
            &webhooks.url,
        ]);
    if !create_arguments.is_empty() {
        command.arg("--").args(create_arguments);
    }
    Watcher {
        child: command.spawn().unwrap(),
    }
}

fn run_ffmpeg(arguments: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(arguments)
        .status()
        .expect("ffmpeg is required by the watch folder tests");
    assert!(status.success(), "ffmpeg failed: {arguments:?}");
}

fn make_test_video(directory: &Path, file_name: &str) -> PathBuf {
    let path = directory.join(file_name);
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1920x1080:rate=24",
        "-frames:v",
        "12",
        "-pix_fmt",
        "yuv420p",
        path.to_str().unwrap(),
    ]);
    path
}

fn make_silent_wav(directory: &Path, file_name: &str) -> PathBuf {
    let path = directory.join(file_name);
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=48000:cl=stereo",
        "-t",
        "0.5",
        path.to_str().unwrap(),
    ]);
    path
}

fn wait_for(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(CHECK_INTERVAL);
    }
}

fn file_named(directory: &Path, prefix: &str, extension: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(directory).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) && name.ends_with(extension) {
            return Some(entry.path());
        }
    }
    None
}

#[test]
fn a_master_that_lands_becomes_an_imp() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let mut watcher = start_watcher(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
    );

    let video = make_test_video(scratch.path(), "feature.mp4");
    let audio = make_silent_wav(scratch.path(), "feature.wav");
    std::fs::rename(&audio, watch_dir.path().join("feature.wav")).unwrap();
    std::fs::rename(&video, watch_dir.path().join("feature.mp4")).unwrap();

    let package_dir = output_dir.path().join("feature");
    assert!(
        wait_for(&package_dir.join("ASSETMAP.xml"), BUILD_TIMEOUT),
        "no ASSETMAP.xml under {}",
        package_dir.display()
    );
    assert!(
        wait_for(
            &watch_dir.path().join("done").join("feature.mp4"),
            BUILD_TIMEOUT
        ),
        "the master was not moved into done/"
    );

    let cpl = file_named(&package_dir, "CPL_", ".xml").expect("no CPL in the package");
    let cpl_text = std::fs::read_to_string(&cpl).unwrap();
    assert!(
        cpl_text.contains("<ContentTitle>feature</ContentTitle>"),
        "the CPL does not carry the file stem as its title"
    );
    assert!(
        file_named(&package_dir, "AUDIO_", ".mxf").is_some(),
        "the sidecar wav was not packaged as sound"
    );
    assert!(
        watch_dir.path().join("done").join("feature.wav").exists(),
        "the sidecar wav was not moved into done/"
    );

    let log = std::fs::read_to_string(output_dir.path().join("feature.log")).unwrap();
    assert!(!log.is_empty(), "the job log is empty");

    let created = webhooks.wait_for_one("imp.created");
    assert_eq!(
        created.len(),
        1,
        "expected one imp.created body, got {created:?}"
    );

    watcher.child.kill().unwrap();
    let status = watcher.child.wait().unwrap();
    assert!(!status.success(), "the watcher was not killed");
    assert!(
        watcher.child.try_wait().unwrap().is_some(),
        "the watcher is still running"
    );
}

#[test]
fn a_master_create_refuses_lands_in_failed() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let _watcher = start_watcher(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
    );

    let broken = scratch.path().join("broken.mov");
    std::fs::write(&broken, [0u8; 100]).unwrap();
    std::fs::rename(&broken, watch_dir.path().join("broken.mov")).unwrap();

    assert!(
        wait_for(
            &watch_dir.path().join("failed").join("broken.mov"),
            FAILURE_TIMEOUT
        ),
        "the master was not moved into failed/"
    );

    let log = std::fs::read_to_string(output_dir.path().join("broken.log")).unwrap();
    assert!(
        log.contains("cannot read the picture size"),
        "the job log does not name the failure: {log}"
    );

    let failed = webhooks.wait_for_one("imp.failed");
    assert_eq!(
        failed.len(),
        1,
        "expected one imp.failed body, got {failed:?}"
    );
}

/// A one cue IMSC document, the subtitle sidecar the watcher looks for.
fn make_ttml(directory: &Path, file_name: &str) -> PathBuf {
    let path = directory.join(file_name);
    std::fs::write(
        &path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en">
  <body><div>
    <p begin="00:00:00.100" end="00:00:00.400">Sidecar cue</p>
  </div></body>
</tt>"#,
    )
    .unwrap();
    path
}

/// A `.ttml` beside the master is packaged as the composition's timed text and
/// moved into `done/` with it, the same way the `.wav` sidecar is.
#[test]
fn a_ttml_sidecar_lands_in_the_package() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let _watcher = start_watcher(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
    );

    let video = make_test_video(scratch.path(), "subtitled.mp4");
    make_ttml(watch_dir.path(), "subtitled.ttml");
    std::fs::rename(&video, watch_dir.path().join("subtitled.mp4")).unwrap();

    let package_dir = output_dir.path().join("subtitled");
    assert!(
        wait_for(&package_dir.join("ASSETMAP.xml"), BUILD_TIMEOUT),
        "no ASSETMAP.xml under {}",
        package_dir.display()
    );

    assert!(
        file_named(&package_dir, "SUBTITLE_", ".mxf").is_some(),
        "the ttml sidecar was not wrapped: {:?}",
        std::fs::read_dir(&package_dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>()
    );

    let cpl = file_named(&package_dir, "CPL_", ".xml").expect("no CPL in the package");
    let cpl_text = std::fs::read_to_string(&cpl).unwrap();
    assert!(
        cpl_text.contains("SubtitlesSequence"),
        "the CPL registers no timed text track: {cpl_text}"
    );

    assert!(
        wait_for(
            &watch_dir.path().join("done").join("subtitled.ttml"),
            BUILD_TIMEOUT
        ),
        "the ttml sidecar was not moved into done/"
    );
}

// with this interval the watcher needs two polls to call a master settled, so
// nothing can be built before twice it has passed
const SLOW_INTERVAL_SECONDS: u64 = 6;

/// `--interval` is the poll period, and a master is only built once two polls
/// have measured it the same: a slow interval holds the build back, and the
/// same master under the default interval is built while the slow one waits.
#[test]
fn a_slow_interval_holds_the_build_back() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let _watcher = start_watcher_with(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
        &SLOW_INTERVAL_SECONDS.to_string(),
        &[],
    );

    let video = make_test_video(scratch.path(), "slow.mp4");
    std::fs::rename(&video, watch_dir.path().join("slow.mp4")).unwrap();
    let landed = Instant::now();

    let log = output_dir.path().join("slow.log");
    // the log is created as the build starts, so it dates the first build
    assert!(
        wait_for(&log, BUILD_TIMEOUT),
        "the watcher never started a build"
    );
    let waited = landed.elapsed();
    assert!(
        waited >= Duration::from_secs(SLOW_INTERVAL_SECONDS),
        "the build started after {waited:?}, inside one {SLOW_INTERVAL_SECONDS} s poll"
    );
}

/// Flags after `--` are appended to every `create` the watcher runs, so a
/// watched folder can deliver at a named raster and rate rather than the
/// defaults.
#[test]
fn passthrough_flags_after_the_dashes_reach_the_build() {
    const RASTER_WIDTH: u32 = 2048;
    const RASTER_HEIGHT: u32 = 1080;

    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let _watcher = start_watcher_with(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
        POLL_INTERVAL_SECONDS,
        &[
            "--raster",
            &format!("{RASTER_WIDTH}x{RASTER_HEIGHT}"),
            "--kind",
            "trailer",
        ],
    );

    let video = make_test_video(scratch.path(), "passthrough.mp4");
    std::fs::rename(&video, watch_dir.path().join("passthrough.mp4")).unwrap();

    let package_dir = output_dir.path().join("passthrough");
    assert!(
        wait_for(&package_dir.join("ASSETMAP.xml"), BUILD_TIMEOUT),
        "no ASSETMAP.xml under {}: {}",
        package_dir.display(),
        std::fs::read_to_string(output_dir.path().join("passthrough.log")).unwrap_or_default()
    );

    // the source is 1920x1080, so this raster only appears if the flag arrived
    let picture = file_named(&package_dir, "VIDEO_", ".mxf").expect("no picture track file");
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader.open_read(picture.to_str().unwrap()).unwrap();
    let descriptor = reader.picture_descriptor().unwrap();
    reader.close().unwrap();
    assert_eq!(
        (descriptor.stored_width, descriptor.stored_height),
        (RASTER_WIDTH, RASTER_HEIGHT),
        "--raster did not reach the build"
    );

    let cpl = file_named(&package_dir, "CPL_", ".xml").expect("no CPL in the package");
    let cpl_text = std::fs::read_to_string(&cpl).unwrap();
    assert!(
        cpl_text.contains("<ContentKind>trailer</ContentKind>"),
        "--kind did not reach the build: {cpl_text}"
    );
}
