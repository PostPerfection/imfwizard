import json
import os
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

TAURI_DRIVER = Path(
    os.environ.get("TAURI_DRIVER", Path.home() / ".cargo/bin/tauri-driver")
)

DRIVER_READY_TIMEOUT_SECONDS = 60
DRIVER_EXIT_TIMEOUT_SECONDS = 10
REQUEST_TIMEOUT_SECONDS = 180
POLL_SECONDS = 0.2

APPLICATION_PROCESS_NAME = "imfwizard-gui"

# tauri maps a second window carrying the binary name, a few pixels wide
MIN_WINDOW_SIDE = 200
WINDOW_TIMEOUT_SECONDS = 60
TYPE_DELAY_MILLISECONDS = 20
INPUT_SETTLE_SECONDS = 0.3

FILE_DIALOG_TIMEOUT_SECONDS = 60
# the picker opens on its recent list, where confirming a typed path does nothing
FILE_DIALOG_HOME_CHORD = "alt+Home"
FILE_DIALOG_LOCATION_CHORD = "ctrl+l"
# the picker completes a typed folder name with a trailing slash, and confirming
# that walks into the folder instead of choosing it
DROP_COMPLETION_KEY = "Delete"
CONFIRM_KEY = "Return"

# viewport centre after scrolling the element into view
CENTRE_OF_ELEMENT = """
const element = document.querySelector(arguments[0]);
if (!element) return null;
element.scrollIntoView({ block: "center", inline: "center" });
const box = element.getBoundingClientRect();
if (!box.width || !box.height) return null;
return [box.left + box.width / 2, box.top + box.height / 2];
"""


class WebDriverError(RuntimeError):
    pass


def visible_windows():
    found = subprocess.run(
        ("xdotool", "search", "--onlyvisible", "--name", "."),
        capture_output=True,
        text=True,
    )
    return set(found.stdout.split())


def xdotool(*arguments):
    done = subprocess.run(("xdotool",) + arguments, capture_output=True, text=True)
    if done.returncode != 0:
        raise WebDriverError(f"xdotool {' '.join(arguments)}: {done.stderr.strip()}")
    return done.stdout


# WebKitWebDriver does not spell the web element key the way the spec does
def element_handle(found):
    return next(iter(found.values()))


def request(method, url, body=None):
    data = json.dumps(body).encode() if body is not None else None
    call = urllib.request.Request(
        url, data=data, method=method, headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(call, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            payload = json.loads(response.read())
    except urllib.error.HTTPError as failure:
        detail = failure.read().decode(errors="replace")
        raise WebDriverError(f"{method} {url}: {detail[:600]}") from None
    return payload["value"]


def free_port():
    with socket.socket() as held:
        held.bind(("127.0.0.1", 0))
        return held.getsockname()[1]


def wait_until(describe, check, timeout_seconds):
    deadline = time.monotonic() + timeout_seconds
    dropped = None
    while True:
        try:
            result = check()
        except OSError as connection_error:
            # the driver drops a connection now and then, and a poll can retry
            result, dropped = None, connection_error
        if result:
            return result
        if time.monotonic() >= deadline:
            reason = f", last: {dropped}" if dropped else ""
            raise AssertionError(f"{describe} within {timeout_seconds}s{reason}")
        time.sleep(POLL_SECONDS)


class Session:
    def __init__(self, url, session_id):
        self.url = url
        self.session_id = session_id

    def call(self, method, path, body=None):
        return request(method, f"{self.url}/session/{self.session_id}{path}", body)

    def find(self, css):
        try:
            found = self.call("POST", "/element", {"using": "css selector", "value": css})
        except WebDriverError:
            return None
        return element_handle(found)

    def element(self, css):
        found = self.find(css)
        if found is None:
            raise AssertionError(f"no element matches {css}")
        return found

    def text(self, css):
        return self.call("GET", f"/element/{self.element(css)}/text")

    def property(self, css, name):
        return self.call("GET", f"/element/{self.element(css)}/property/{name}")

    def attribute(self, css, name):
        return self.call("GET", f"/element/{self.element(css)}/attribute/{name}")

    def execute(self, script, *arguments):
        return self.call("POST", "/execute/sync", {"script": script, "args": list(arguments)})

    def close(self):
        self.call("DELETE", "")


class Window:
    def __init__(self, binary, environment, log_path, window_title):
        self.environment = dict(environment)
        self.window_title = window_title
        self.window_id = None
        self.origin = (0, 0)
        self.log_path = Path(log_path)
        self.url = f"http://127.0.0.1:{free_port()}"
        self.log = self.log_path.open("wb")
        self.driver = subprocess.Popen(
            [
                str(TAURI_DRIVER),
                "--port",
                self.url.rsplit(":", 1)[1],
                "--native-port",
                str(free_port()),
            ],
            env=self.environment,
            stdout=self.log,
            stderr=subprocess.STDOUT,
        )
        self.session = None
        try:
            self._wait_for_driver()
            started = request(
                "POST",
                f"{self.url}/session",
                {
                    "capabilities": {
                        "alwaysMatch": {
                            "tauri:options": {"application": str(binary)},
                            "browserName": "wry",
                        }
                    }
                },
            )
            self.session = Session(self.url, started["sessionId"])
        except Exception:
            self.close()
            raise

    def _wait_for_driver(self):
        deadline = time.monotonic() + DRIVER_READY_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            if self.driver.poll() is not None:
                raise WebDriverError(
                    f"tauri-driver exited with {self.driver.returncode}\n{self.output()}"
                )
            try:
                request("GET", f"{self.url}/status")
                return
            except (urllib.error.URLError, OSError):
                time.sleep(POLL_SECONDS)
        raise WebDriverError(f"tauri-driver was not listening on {self.url}")

    def take_focus(self, neutral_target):
        self.window_id = wait_until(
            "no app window appeared", self._app_window, WINDOW_TIMEOUT_SECONDS
        )
        xdotool("windowfocus", "--sync", self.window_id)
        # the webview reads no key until a click has landed in the page
        self.click(neutral_target)

    def _app_window(self):
        found = xdotool("search", "--onlyvisible", "--name", self.window_title)
        for window_id in found.split():
            geometry = self._geometry(window_id)
            if min(geometry["WIDTH"], geometry["HEIGHT"]) >= MIN_WINDOW_SIDE:
                self.origin = (geometry["X"], geometry["Y"])
                return window_id
        return None

    def _geometry(self, window_id):
        shell = xdotool("getwindowgeometry", "--shell", window_id)
        pairs = (line.split("=", 1) for line in shell.splitlines() if "=" in line)
        return {name: int(value) for name, value in pairs}

    # WebKitWebDriver refuses element click, value and actions in a wry webview
    def click(self, css):
        spot = self.session.execute(CENTRE_OF_ELEMENT, css)
        if spot is None:
            raise AssertionError(f"{css} has no box on the page to click")
        # no --sync: a move to where the pointer already sits reports no motion
        xdotool(
            "mousemove",
            str(self.origin[0] + round(spot[0])),
            str(self.origin[1] + round(spot[1])),
            "click",
            "1",
        )
        time.sleep(INPUT_SETTLE_SECONDS)

    # with no window manager under Xvfb the picker reads keys only once focused
    def answer_file_dialog(self, path, windows_before):
        dialog = wait_until(
            "no file dialog opened",
            lambda: next(iter(visible_windows() - windows_before), None),
            FILE_DIALOG_TIMEOUT_SECONDS,
        )
        xdotool("windowfocus", "--sync", dialog)
        self.press(FILE_DIALOG_HOME_CHORD)
        self.press(FILE_DIALOG_LOCATION_CHORD)
        self.type_text(str(path))
        self.press(DROP_COMPLETION_KEY)
        self.press(CONFIRM_KEY)
        wait_until(
            "the file dialog stayed open",
            lambda: dialog not in visible_windows(),
            FILE_DIALOG_TIMEOUT_SECONDS,
        )
        xdotool("windowfocus", "--sync", self.window_id)

    def type_text(self, text):
        xdotool("type", "--delay", str(TYPE_DELAY_MILLISECONDS), text)
        time.sleep(INPUT_SETTLE_SECONDS)

    def press(self, chord):
        xdotool("key", "--clearmodifiers", chord)
        time.sleep(INPUT_SETTLE_SECONDS)

    def output(self):
        return (
            self.log_path.read_text(errors="replace")[-4000:]
            if self.log_path.exists()
            else ""
        )

    def close(self):
        if self.session is not None:
            try:
                self.session.close()
            except (WebDriverError, urllib.error.URLError, OSError):
                pass
            self.session = None
        if self.driver.poll() is None:
            self.driver.terminate()
            try:
                self.driver.wait(timeout=DRIVER_EXIT_TIMEOUT_SECONDS)
            except subprocess.TimeoutExpired:
                self.driver.kill()
                self.driver.wait(timeout=DRIVER_EXIT_TIMEOUT_SECONDS)
        self.log.close()
        kill_leftover_apps(self.environment["XDG_CONFIG_HOME"])


# only this session's config dir tells its window apart from any other
def kill_leftover_apps(config_home):
    marker = f"XDG_CONFIG_HOME={config_home}".encode()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if APPLICATION_PROCESS_NAME not in (entry / "comm").read_text():
                continue
            if marker not in (entry / "environ").read_bytes():
                continue
            os.kill(int(entry.name), 9)
        except OSError:
            continue
