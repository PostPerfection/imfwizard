import base64
import hashlib
import json
import os
import re
import shutil
import subprocess
import uuid
import xml.etree.ElementTree as ElementTree
from pathlib import Path

import pytest

from tauri_webdriver import Window, visible_windows, wait_until

REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_GUI_BINARY = REPOSITORY_ROOT / "gui/src-tauri/target/release/imfwizard-gui"
DEFAULT_CLI_BINARY = REPOSITORY_ROOT / "rust/target/release/imfwizard"
WINDOW_TITLE = "IMF Wizard"

# the heading has no click handler, a click there only focuses the webview
NEUTRAL_TARGET = ".toolbar-left h1"

PAGE_TIMEOUT_SECONDS = 60
IMPORT_TIMEOUT_SECONDS = 60
HINTS_TIMEOUT_SECONDS = 180
BUILD_TIMEOUT_SECONDS = 300
PREVIEW_TIMEOUT_SECONDS = 60
STATUS_TIMEOUT_SECONDS = 60
RESTART_TIMEOUT_SECONDS = 60
REVEAL_TIMEOUT_SECONDS = 60
OPEN_TIMEOUT_SECONDS = 60
PLAYHEAD_TIMEOUT_SECONDS = 60
CREATE_TIMEOUT_SECONDS = 900
# a click or a key press is answered in the page, not over the network
REACTION_TIMEOUT_SECONDS = 15

BUILD_TITLE = "Wizard End To End"

# both printed by imfwizard create --check on the media write_media makes
LANGUAGE_HINT = "The sound has no language set. Set one unless it has no spoken parts."
AUDIO_LEVEL_HINT_PREFIX = "The audio level is very high"

FINISHED_STAGES = ("Done", "Cancelled", "Failed", "Error")
DONE_STAGE = "Done"
ENCODE_STAGE = "Encode"
BUILD_NOTIFICATION_TITLE = "Build Complete"

SHORTCUTS_STORAGE_KEY = "imfwizard-shortcuts"
TIMELINE_ACTION = "view-timeline"
TIMELINE_LABEL = "Timeline"
TIMELINE_DEFAULT_CHORD = "Ctrl+2"
REBOUND_CHORD = "Ctrl+9"
CAPTURE_PROMPT = "press new shortcut"
SHORTCUT_TRIGGER_ID = "test-shortcut-trigger"

GPU_UNAVAILABLE_PREFIX = "GPU encoding unavailable"

FIXTURE_SIZE = "1920x1080"
FIXTURE_FPS = 24
BUILD_MEDIA_SECONDS = 1
PLAYBACK_MEDIA_SECONDS = 4
# the sine peaks at -18 dBTP, the level hint wants more than -3
SOUND_GAIN_DB = 17

PLAYBACK_TITLE = "Playback End To End"
TEXT_DIALOG = ".text-dialog"
PREVIEW_DEFAULT_TITLE = "Preview"
# playing a span takes longer than the span lasts when decoding lags
PLAYBACK_TIMEOUT_MULTIPLE = 3
# the tick to seek to, as how far along the ruler it has to sit at least
SEEK_TICK_LEFT_PERCENT = 20
SEEK_TICK_ID = "test-seek-tick"

XDG_DIRECTORIES = {
    "XDG_CONFIG_HOME": "config",
    "XDG_DATA_HOME": "data",
    "XDG_CACHE_HOME": "cache",
}

# the OS notification is outside the app, so the constructor is recorded
RECORD_NOTIFICATIONS = """
window.__testNotifications = [];
window.Notification = function (title, options) {
  window.__testNotifications.push({ title, options });
};
window.Notification.permission = "granted";
window.Notification.requestPermission = () => Promise.resolve("granted");
return typeof window.Notification;
"""

# reveal goes over D-Bus and no file manager runs here, the refusal is the trace
RECORD_REJECTIONS = """
window.__testRejections = [];
window.addEventListener("unhandledrejection", (event) => {
  window.__testRejections.push(String(event.reason));
});
return true;
"""

PROGRESS_STATE = """
return {
  value: document.getElementById("progress-bar").value,
  stage: document.getElementById("progress-stage").textContent,
  stats: document.getElementById("progress-stats").textContent,
  visible: document.getElementById("progress-section").style.display !== "none",
};
"""

# css cannot pick a row out by its label, so the row's button is given an id
MARK_SHORTCUT_TRIGGER = """
const row = [...document.querySelectorAll(".shortcuts-row")].find(
  (candidate) => candidate.querySelector(".shortcuts-label").textContent === arguments[0],
);
if (!row) return null;
const trigger = row.querySelector(".shortcuts-binding");
trigger.id = arguments[1];
return trigger.textContent;
"""

JOBS_ROWS = """
return [...document.querySelectorAll("#jobs-tbody tr")].map((row) => {
  const cells = [...row.querySelectorAll("td")].map((cell) => cell.textContent);
  return { id: cells[0], source: cells[1], title: cells[2], state: cells[3] };
});
"""

RECENT_PATHS = """
return [...document.querySelectorAll("#recent-list .recent-item")].map(
  (item) => item.dataset.path,
);
"""

SEGMENTS = """
return [...document.querySelectorAll(arguments[0] + " .timeline-segment")].map(
  (segment) => ({
    index: segment.dataset.segIndex,
    duration: segment.querySelector(".segment-duration").textContent,
    active: segment.classList.contains("active"),
  }),
);
"""

PLAYHEAD_LEFT = """
const playhead = document.getElementById("ruler-playhead");
return playhead ? parseFloat(playhead.style.left) : null;
"""

# css cannot pick a tick out by where it sits, so the one to click is given an id
MARK_TICK = """
const ticks = [...document.querySelectorAll("#timeline-ruler .ruler-tick")];
const wanted = ticks.find((tick) => parseFloat(tick.style.left) >= arguments[0]);
if (!wanted) return null;
wanted.id = arguments[1];
return wanted.style.left;
"""


def gui_binary():
    binary = Path(os.environ.get("IMFWIZARD_GUI", DEFAULT_GUI_BINARY))
    assert binary.is_file(), f"GUI binary not found at {binary}"
    return binary


def cli_binary():
    binary = Path(os.environ.get("IMFWIZARD_CLI", DEFAULT_CLI_BINARY))
    assert binary.is_file(), f"CLI binary not found at {binary}"
    return binary


def application_environment(root):
    environment = dict(os.environ)
    # the app runs under XWayland on a wayland desktop, and under Xvfb in CI
    environment["GDK_BACKEND"] = "x11"
    for name, directory in XDG_DIRECTORIES.items():
        (root / directory).mkdir(parents=True, exist_ok=True)
        environment[name] = str(root / directory)
    # no session bus, so Reveal cannot reach the desktop's own file manager
    runtime = root / "runtime"
    runtime.mkdir(mode=0o700, exist_ok=True)
    environment.pop("DBUS_SESSION_BUS_ADDRESS", None)
    environment["XDG_RUNTIME_DIR"] = str(runtime)
    environment["IMFWIZARD_GUI_JOBS_FILE"] = str(root / "gui-jobs.jsonl")
    return environment


def open_window(environment, log_path):
    assert os.environ.get("DISPLAY"), "no DISPLAY: run under xvfb-run or a desktop"
    window = Window(gui_binary(), environment, log_path, WINDOW_TITLE)
    try:
        wait_until(
            "the page never loaded",
            lambda: window.session.find("#theme-toggle"),
            PAGE_TIMEOUT_SECONDS,
        )
        window.take_focus(NEUTRAL_TARGET)
    except Exception:
        window.close()
        raise
    return window


# the picture and sound the CLI's own conformance test makes
def write_media(directory, seconds):
    picture = directory / "source.mov"
    sound = directory / "sound.wav"
    run_ffmpeg(
        "-f", "lavfi",
        "-i", f"testsrc2=size={FIXTURE_SIZE}:rate={FIXTURE_FPS}:duration={seconds}",
        "-f", "lavfi",
        "-i", f"sine=frequency=440:sample_rate=48000:duration={seconds}",
        "-frames:v", str(FIXTURE_FPS * seconds),
        "-c:v", "ffv1", "-c:a", "pcm_s24le", "-shortest",
        str(picture),
    )
    run_ffmpeg(
        "-f", "lavfi",
        "-i", f"sine=frequency=1000:sample_rate=48000:duration={seconds}",
        "-af", f"volume={SOUND_GAIN_DB}dB",
        "-c:a", "pcm_s24le",
        str(sound),
    )
    return picture, sound


def run_ffmpeg(*arguments):
    made = subprocess.run(
        ("ffmpeg", "-y", "-v", "error", *arguments), capture_output=True, text=True
    )
    assert made.returncode == 0, made.stderr


def active_view(session):
    return session.execute("return document.querySelector('.view.active')?.id")


def wait_for_view(session, view):
    wait_until(
        f"{view} never became the active view",
        lambda: active_view(session) == view,
        REACTION_TIMEOUT_SECONDS,
    )


def overlay_hidden(session):
    return session.execute("return document.querySelector('.shortcuts-overlay').hidden")


def wait_for_overlay(session, hidden):
    wait_until(
        f"the shortcut list never became {'hidden' if hidden else 'visible'}",
        lambda: overlay_hidden(session) is hidden,
        REACTION_TIMEOUT_SECONDS,
    )


def body_classes(session):
    return session.property("body", "className").split()


# the footer is clipped in a small window, so the rendered text reads empty
def status_text(session):
    return session.property("#status-text", "textContent")


def track_name(session, track):
    return session.execute(
        f"return document.querySelector('.track-{track} .track-info').textContent"
    )


# the real GTK file picker, driven through its location bar
def choose_in_dialog(window, button, path):
    windows_before = visible_windows()
    window.click(button)
    window.answer_file_dialog(path, windows_before)


def hint_texts(session):
    wait_until(
        "the hints dialog never opened",
        lambda: session.property("#hints-dialog", "hidden") is False,
        HINTS_TIMEOUT_SECONDS,
    )
    return session.execute(
        "return [...document.querySelectorAll('#hints-list li')].map(item => item.textContent)"
    )


def stored_jobs(jobs_file):
    records = {}
    for line in jobs_file.read_text().splitlines():
        record = json.loads(line)
        records[record["config"]["id"]] = record
    return list(records.values())


@pytest.fixture
def window(tmp_path):
    opened = open_window(application_environment(tmp_path), tmp_path / "driver.log")
    yield opened
    # pytest shows this only when the test failed
    print(opened.output())
    opened.close()


def test_the_theme_toggle_switches_the_body_class_and_the_button(window):
    session = window.session
    assert session.text("#theme-toggle") == "🌙"

    window.click("#theme-toggle")
    wait_until(
        "the light theme never came on",
        lambda: "light" in body_classes(session),
        REACTION_TIMEOUT_SECONDS,
    )
    assert session.text("#theme-toggle") == "☀️"

    window.click("#theme-toggle")
    wait_until(
        "the dark theme never came back",
        lambda: "light" not in body_classes(session),
        REACTION_TIMEOUT_SECONDS,
    )
    assert session.text("#theme-toggle") == "🌙"


def test_the_shortcuts_switch_views_open_the_list_and_rebind(window):
    session = window.session

    window.press("ctrl+2")
    wait_for_view(session, "view-timeline")
    window.press("ctrl+1")
    wait_for_view(session, "view-project")

    window.press("ctrl+k")
    wait_for_overlay(session, hidden=False)
    window.press("Escape")
    wait_for_overlay(session, hidden=True)

    window.press("ctrl+k")
    wait_for_overlay(session, hidden=False)
    bound = session.execute(MARK_SHORTCUT_TRIGGER, TIMELINE_LABEL, SHORTCUT_TRIGGER_ID)
    assert bound == TIMELINE_DEFAULT_CHORD
    window.click(f"#{SHORTCUT_TRIGGER_ID}")
    assert CAPTURE_PROMPT in session.execute(
        "return [...document.querySelectorAll('.shortcuts-binding')].map(b => b.textContent)"
    )

    window.press("ctrl+9")
    stored = wait_until(
        "the rebinding was never stored",
        lambda: json.loads(
            session.execute("return localStorage.getItem(arguments[0])", SHORTCUTS_STORAGE_KEY)
            or "{}"
        ),
        REACTION_TIMEOUT_SECONDS,
    )
    assert stored[TIMELINE_ACTION] == REBOUND_CHORD

    window.press("Escape")
    wait_for_overlay(session, hidden=True)
    window.press("ctrl+9")
    wait_for_view(session, "view-timeline")


class FinishedBuild:
    def __init__(self, window, root, environment, output):
        self.window = window
        self.root = root
        self.environment = environment
        self.output = output
        self.jobs_file = Path(environment["IMFWIZARD_GUI_JOBS_FILE"])
        self.first_hints = []
        self.progress_after_going_back = None
        self.jobs_file_after_going_back = None
        self.second_hints = []
        self.samples = []
        self.notifications = []


# one build for the two tests below, an assertion here reports as an error
@pytest.fixture(scope="module")
def finished_build(tmp_path_factory):
    root = tmp_path_factory.mktemp("build")
    picture, sound = write_media(root, BUILD_MEDIA_SECONDS)
    output = root / "package"
    output.mkdir()
    environment = application_environment(root)
    window = open_window(environment, root / "driver.log")
    build = FinishedBuild(window, root, environment, output)
    try:
        run_the_build(build, picture, sound)
    except Exception:
        window.close()
        raise
    yield build
    window.close()


def run_the_build(build, picture, sound):
    window = build.window
    session = window.session

    # the app asked for the permission on load, so it can raise a notification
    assert session.execute("return Notification.permission") == "granted"
    session.execute(RECORD_NOTIFICATIONS)

    choose_in_dialog(window, "#import-video", picture)
    wait_until(
        "the video never reached the picture track",
        lambda: track_name(session, "picture") == picture.name,
        IMPORT_TIMEOUT_SECONDS,
    )

    choose_in_dialog(window, "#import-audio", sound)
    wait_until(
        "the audio never reached the sound track",
        lambda: track_name(session, "sound") == sound.name,
        IMPORT_TIMEOUT_SECONDS,
    )

    window.click("#prop-title")
    window.type_text(BUILD_TITLE)

    choose_in_dialog(window, "#browse-output", build.output)
    wait_until(
        "the output folder never took",
        lambda: session.property("#prop-output", "value") == str(build.output),
        IMPORT_TIMEOUT_SECONDS,
    )

    window.click("#btn-build")
    build.first_hints = hint_texts(session)
    window.click("#hints-back")
    wait_until(
        "the hints dialog stayed up",
        lambda: session.property("#hints-dialog", "hidden") is True,
        STATUS_TIMEOUT_SECONDS,
    )
    build.progress_after_going_back = session.execute(PROGRESS_STATE)
    build.jobs_file_after_going_back = build.jobs_file.exists()

    window.click("#btn-build")
    build.second_hints = hint_texts(session)
    window.click("#hints-build")

    def finished():
        build.samples.append(session.execute(PROGRESS_STATE))
        return build.samples[-1]["stage"] in FINISHED_STAGES

    wait_until("the build never finished", finished, BUILD_TIMEOUT_SECONDS)
    build.notifications = session.execute("return window.__testNotifications")


def test_the_build_shows_hints_progress_and_the_post_build_actions(finished_build):
    session = finished_build.window.session

    assert LANGUAGE_HINT in finished_build.first_hints
    assert any(
        hint.startswith(AUDIO_LEVEL_HINT_PREFIX) for hint in finished_build.first_hints
    )
    assert finished_build.progress_after_going_back["visible"] is False
    assert finished_build.jobs_file_after_going_back is False
    assert finished_build.second_hints == finished_build.first_hints

    last = finished_build.samples[-1]
    assert last["stage"] == DONE_STAGE, finished_build.samples[-3:]
    assert last["value"] == 100
    stages = [sample["stage"] for sample in finished_build.samples]
    assert ENCODE_STAGE in stages, stages
    assert any(
        0 < sample["value"] < 100 and sample["stats"] for sample in finished_build.samples
    ), finished_build.samples

    assert [notification["title"] for notification in finished_build.notifications] == [
        BUILD_NOTIFICATION_TITLE
    ]

    assert session.property("#post-build-actions", "hidden") is False
    assert session.attribute("#post-build-actions", "data-output") == str(
        finished_build.output
    )
    assert (finished_build.output / "ASSETMAP.xml").is_file()

    assert session.execute(RECENT_PATHS) == [str(finished_build.output)]

    session.execute(RECORD_REJECTIONS)
    assert session.execute("return window.__testRejections") == []
    finished_build.window.click("#post-build-reveal")
    refusals = wait_until(
        "the reveal never reached the desktop",
        lambda: session.execute("return window.__testRejections"),
        REVEAL_TIMEOUT_SECONDS,
    )
    assert len(refusals) == 1, refusals

    finished_build.window.click("#post-build-inspect")
    wait_for_view(session, "view-validate")
    assert session.property("#val-path", "textContent") == str(finished_build.output)

    finished_build.window.click("#post-build-play")
    wait_until(
        "the preview panel stayed hidden",
        lambda: session.property("#preview-panel", "hidden") is False,
        PREVIEW_TIMEOUT_SECONDS,
    )
    wait_until(
        "the preview never reported a duration",
        lambda: float(session.attribute("#timeline-duration", "data-raw")) > 0,
        PREVIEW_TIMEOUT_SECONDS,
    )

    jobs = stored_jobs(finished_build.jobs_file)
    assert [job["state"] for job in jobs] == ["Done"]
    assert jobs[0]["config"]["title"] == BUILD_TITLE


def test_the_queue_the_recent_list_and_the_theme_come_back_after_a_restart(finished_build):
    finished_build.window.click("#theme-toggle")
    preferences_file = finished_build.root / XDG_DIRECTORIES["XDG_CONFIG_HOME"] / "imfwizard/preferences.json"
    wait_until(
        "the light theme was never saved",
        lambda: preferences_file.is_file() and json.loads(preferences_file.read_text())["theme"] == "light",
        STATUS_TIMEOUT_SECONDS,
    )
    finished_build.window.close()
    window = open_window(finished_build.environment, finished_build.root / "restart.log")
    try:
        session = window.session
        window.press("ctrl+6")
        wait_for_view(session, "view-jobs")

        # the panel fills the table with a placeholder row until the jobs land
        rows = wait_until(
            "the jobs table never listed the build",
            lambda: [row for row in session.execute(JOBS_ROWS) if row["state"]],
            RESTART_TIMEOUT_SECONDS,
        )
        assert [(row["title"], row["state"]) for row in rows] == [(BUILD_TITLE, "done")]

        window.press("ctrl+1")
        wait_for_view(session, "view-project")
        assert session.execute(RECENT_PATHS) == [str(finished_build.output)]
        assert "light" in body_classes(session)
        assert session.text("#theme-toggle") == "☀️"
    finally:
        window.close()


# the success path needs grok's plugin and a licence, hand tested
def test_the_gpu_toggle_reports_the_missing_plugin_and_stays_off(window, tmp_path):
    session = window.session
    window.press("ctrl+7")
    wait_for_view(session, "view-settings")

    window.click("#set-gpu-enable")
    status = wait_until(
        "the status never mentioned the GPU",
        lambda: status_text(session).startswith(GPU_UNAVAILABLE_PREFIX)
        and status_text(session),
        STATUS_TIMEOUT_SECONDS,
    )
    assert GPU_UNAVAILABLE_PREFIX in status
    assert session.property("#set-gpu-enable", "checked") is False

    preferences_file = tmp_path / XDG_DIRECTORIES["XDG_CONFIG_HOME"] / "imfwizard/preferences.json"
    wait_until(
        "the preferences were never written",
        preferences_file.is_file,
        STATUS_TIMEOUT_SECONDS,
    )
    assert json.loads(preferences_file.read_text())["gpu"] is False


ID_ELEMENT = re.compile(r"<Id>urn:uuid:[0-9a-fA-F-]+</Id>")
SEGMENT_ELEMENT = re.compile(r"[ \t]*<Segment>.*?</Segment>\n", re.DOTALL)


class PlayablePackage:
    def __init__(self, directory):
        self.directory = directory
        self.segments = image_resources(only_cpl(directory))


def only_cpl(directory):
    found = sorted(directory.glob("CPL_*.xml"))
    assert len(found) == 1, found
    return found[0]


def named(parent, name):
    return [node for node in parent.iter() if node.tag.rsplit("}", 1)[-1] == name]


def only_text(parent, name):
    found = named(parent, name)
    assert len(found) == 1, f"{len(found)} {name} elements in {parent.tag}"
    return found[0].text.strip()


# what each segment's picture plays, as (frames, frames per second)
def image_resources(cpl_path):
    resources = []
    for segment in named(ElementTree.parse(cpl_path).getroot(), "Segment"):
        resource = named(named(segment, "MainImageSequence")[0], "Resource")[0]
        numerator, denominator = only_text(resource, "EditRate").split()
        resources.append(
            (
                int(only_text(resource, "SourceDuration")),
                round(int(numerator) / int(denominator)),
            )
        )
    return resources


def sha1_base64(path):
    return base64.b64encode(hashlib.sha1(path.read_bytes()).digest()).decode()


def halves(frames):
    first = frames // 2
    return ((0, first), (first, frames - first))


# the one segment the CLI writes, twice, each half playing the span of the same
# track file the other leaves alone
def halved_segments(segment_xml, frames):
    segments = []
    for entry_point, duration in halves(frames):
        half = ID_ELEMENT.sub(lambda _: f"<Id>urn:uuid:{uuid.uuid4()}</Id>", segment_xml)
        segments.append(
            half.replace(
                f"<SourceDuration>{frames}</SourceDuration>",
                f"<EntryPoint>{entry_point}</EntryPoint>\n"
                f"              <SourceDuration>{duration}</SourceDuration>",
            )
        )
    return "".join(segments)


def write_two_segment_cpl(directory, frames):
    original = only_cpl(directory)
    text = original.read_text()
    composition_id = ID_ELEMENT.search(text).group(0)
    segment = SEGMENT_ELEMENT.search(text).group(0)

    new_id = str(uuid.uuid4())
    text = text.replace(segment, halved_segments(segment, frames))
    text = text.replace(composition_id, f"<Id>urn:uuid:{new_id}</Id>", 1)
    new_cpl = directory / f"CPL_{new_id}.xml"
    new_cpl.write_text(text)
    original.unlink()

    old_id = composition_id.removeprefix("<Id>urn:uuid:").removesuffix("</Id>")
    restate_assetmap(directory, original.name, new_cpl.name, old_id, new_id)
    restate_pkl(directory, new_cpl, old_id, new_id)


# both the app's CPL list and the player read the ASSETMAP, so the halved CPL
# has to be the one it names
def restate_assetmap(directory, old_name, new_name, old_id, new_id):
    assetmap = directory / "ASSETMAP.xml"
    assetmap.write_text(
        assetmap.read_text().replace(old_name, new_name).replace(old_id, new_id)
    )


def restate_pkl(directory, cpl_path, old_id, new_id):
    pkl = next(directory.glob("PKL_*.xml"))
    text = pkl.read_text()
    asset = re.search(
        rf"<Asset>\s*<Id>urn:uuid:{old_id}</Id>.*?</Asset>", text, re.DOTALL
    ).group(0)
    stated = asset.replace(old_id, new_id)
    stated = re.sub(r"<Hash>[^<]*</Hash>", f"<Hash>{sha1_base64(cpl_path)}</Hash>", stated)
    stated = re.sub(r"<Size>\d+</Size>", f"<Size>{cpl_path.stat().st_size}</Size>", stated)
    pkl.write_text(text.replace(asset, stated))


# one encode for both playback tests, an assertion here reports as an error
@pytest.fixture(scope="module")
def single_segment_imp(tmp_path_factory):
    root = tmp_path_factory.mktemp("playback")
    picture, sound = write_media(root, PLAYBACK_MEDIA_SECONDS)
    output = root / "single-segment"
    created = subprocess.run(
        (
            str(cli_binary()),
            "create",
            "--title", PLAYBACK_TITLE,
            "--video", str(picture),
            "--audio", str(sound),
            "--output", str(output),
        ),
        capture_output=True,
        text=True,
        timeout=CREATE_TIMEOUT_SECONDS,
    )
    assert created.returncode == 0, created.stderr[-4000:]
    return PlayablePackage(output)


# the CLI has no split option, so the two segments are written over a copy
@pytest.fixture(scope="module")
def two_segment_imp(single_segment_imp, tmp_path_factory):
    directory = tmp_path_factory.mktemp("crossing") / "two-segment"
    shutil.copytree(single_segment_imp.directory, directory)
    frames, _ = single_segment_imp.segments[0]
    write_two_segment_cpl(directory, frames)
    return PlayablePackage(directory)


# the label timeline.js writes into .segment-duration
def timecode_short(seconds):
    if seconds <= 0:
        return "0:00"
    hours = int(seconds // 3600)
    minutes = int(seconds % 3600 // 60)
    whole_seconds = int(seconds % 60)
    if hours > 0:
        return f"{hours}:{minutes:02d}:{whole_seconds:02d}"
    return f"{minutes}:{whole_seconds:02d}"


def expected_segments(segments):
    return [
        {"index": str(index), "duration": timecode_short(frames / fps)}
        for index, (frames, fps) in enumerate(segments)
    ]


def segments_on_track(session, track):
    return session.execute(SEGMENTS, f"#timeline-{track}")


def listed_segments(session, track):
    found = wait_until(
        f"the {track} track never listed a segment",
        lambda: segments_on_track(session, track) or None,
        OPEN_TIMEOUT_SECONDS,
    )
    return [
        {key: value for key, value in segment.items() if key != "active"}
        for segment in found
    ]


def segment_is_active(session, track, index):
    return any(
        segment["active"] and segment["index"] == str(index)
        for segment in segments_on_track(session, track)
    )


def composition_seconds(segments):
    return sum(frames / fps for frames, fps in segments)


def preview_title(session):
    return session.property("#preview-title", "textContent")


# what the scrubber says the player holds, in seconds
def reported_duration(session):
    return float(session.attribute("#timeline-duration", "data-raw"))


# guikit builds the dialog on the first ask, so before that there is no element
def text_dialog_open(session):
    return session.find(TEXT_DIALOG) is not None and session.property(
        TEXT_DIALOG, "open"
    )


def open_the_timeline(window, package):
    choose_in_dialog(window, "#btn-open-project", package.directory)
    window.press("ctrl+2")
    wait_for_view(window.session, "view-timeline")


def start_the_preview(window):
    session = window.session
    window.click("#btn-preview")
    wait_until(
        "the preview panel stayed hidden",
        lambda: session.property("#preview-panel", "hidden") is False,
        PREVIEW_TIMEOUT_SECONDS,
    )
    wait_until(
        "the preview never reported a duration",
        lambda: reported_duration(session) > 0,
        PREVIEW_TIMEOUT_SECONDS,
    )
    wait_until(
        "the transport never came on",
        lambda: session.property("#timeline-play-btn", "disabled") is False,
        PREVIEW_TIMEOUT_SECONDS,
    )


def test_the_timeline_lists_one_segment_and_plays_the_imp_to_the_end(
    window, single_segment_imp
):
    session = window.session
    segments = single_segment_imp.segments
    assert len(segments) == 1, segments

    open_the_timeline(window, single_segment_imp)
    assert listed_segments(session, "picture") == expected_segments(segments)
    assert listed_segments(session, "sound") == expected_segments(segments)

    assert session.property("#timeline-play-btn", "disabled") is True
    # the panel is hidden until the first load, and a hidden element renders no text
    assert preview_title(session) == PREVIEW_DEFAULT_TITLE

    start_the_preview(window)
    assert preview_title(session).endswith(single_segment_imp.directory.name)
    assert session.property("#btn-preview", "disabled") is True

    started = session.execute(PLAYHEAD_LEFT)
    assert started is not None, "the ruler has no playhead"
    wait_until(
        "the playhead never moved",
        lambda: session.execute(PLAYHEAD_LEFT) > started,
        PLAYHEAD_TIMEOUT_SECONDS,
    )

    whole = composition_seconds(segments)
    frame_seconds = 1 / segments[0][1]
    assert reported_duration(session) == pytest.approx(whole, abs=frame_seconds)

    # once playback runs out the panel holds nothing that is previewing, so the
    # button offers the IMP again
    wait_until(
        "the Preview button never came back at the end of the composition",
        lambda: session.property("#btn-preview", "disabled") is False,
        whole * PLAYBACK_TIMEOUT_MULTIPLE,
    )
    # the play button is how the IMP is played again from the start
    assert session.property("#timeline-play-btn", "disabled") is False
    assert reported_duration(session) == pytest.approx(whole, abs=frame_seconds)

    # the retitle box is the app's own dialog, which the preview surface would
    # cover if it were the webview's prompt
    window.press("ctrl+1")
    wait_for_view(session, "view-project")
    window.click("#recent-header")
    window.click(".recent-retitle")
    wait_until(
        "the retitle dialog never opened",
        lambda: text_dialog_open(session) is True,
        REACTION_TIMEOUT_SECONDS,
    )
    assert (
        session.property(f"{TEXT_DIALOG} input", "value")
        == single_segment_imp.directory.name
    )

    window.press("Escape")
    wait_until(
        "the retitle dialog stayed open",
        lambda: text_dialog_open(session) is False,
        REACTION_TIMEOUT_SECONDS,
    )


def test_the_timeline_follows_playback_across_two_segments(window, two_segment_imp):
    session = window.session
    segments = two_segment_imp.segments
    assert len(segments) == 2, segments

    open_the_timeline(window, two_segment_imp)
    assert listed_segments(session, "picture") == expected_segments(segments)
    assert listed_segments(session, "sound") == expected_segments(segments)

    start_the_preview(window)
    first_frames, first_fps = segments[0]
    wait_until(
        "playback never reached the second segment",
        lambda: segment_is_active(session, "picture", 1),
        first_frames / first_fps * PLAYBACK_TIMEOUT_MULTIPLE,
    )

    frame_seconds = 1 / first_fps
    # one segment's picture loaded behind the panel would halve this duration
    assert reported_duration(session) == pytest.approx(
        composition_seconds(segments), abs=frame_seconds
    )
    assert session.property("#timeline-play-btn", "disabled") is False

    tick = session.execute(MARK_TICK, SEEK_TICK_LEFT_PERCENT, SEEK_TICK_ID)
    assert tick is not None, "the ruler has no tick to seek to"
    window.click(f"#{SEEK_TICK_ID}")
    wait_until(
        f"the seek to {tick} never moved playback back into the first segment",
        lambda: segment_is_active(session, "picture", 0),
        REACTION_TIMEOUT_SECONDS,
    )
    assert reported_duration(session) == pytest.approx(
        composition_seconds(segments), abs=frame_seconds
    )
