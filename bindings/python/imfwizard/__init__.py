"""IMF Wizard — Python bindings for creating IMF packages.

Provides a subprocess-based API that wraps the imfwizard CLI.
Requires the `imfwizard` binary to be on PATH or specified via IMFWIZARD_BIN.
"""

import json
import os
import shutil
import subprocess
from pathlib import Path

__version__ = "1.0.0"


def _find_binary():
    """Locate the imfwizard binary."""
    env_bin = os.environ.get("IMFWIZARD_BIN")
    if env_bin and os.path.isfile(env_bin):
        return env_bin
    found = shutil.which("imfwizard")
    if found:
        return found
    raise FileNotFoundError(
        "imfwizard binary not found. Set IMFWIZARD_BIN or add it to PATH."
    )


def _run(args, check=True):
    """Run imfwizard with given arguments."""
    bin_path = _find_binary()
    result = subprocess.run(
        [bin_path] + args,
        capture_output=True,
        text=True,
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"imfwizard failed (exit {result.returncode}): {result.stderr}"
        )
    return result


def create(title, video, output, audio=None, subtitle=None, preset=None):
    """Create an IMP from source media."""
    args = ["create", "--title", title, "--video", str(video), "--output", str(output)]
    if audio:
        args.extend(["--audio", str(audio)])
    if subtitle:
        args.extend(["--subtitle", str(subtitle)])
    if preset:
        args.extend(["--preset", preset])
    _run(args)
    return Path(output)


def validate(imp_dir):
    """Validate an IMP directory. Returns dict with valid/errors/warnings."""
    result = _run(["validate", "-i", str(imp_dir), "--json"], check=False)
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return {"valid": result.returncode == 0, "errors": [], "warnings": []}


def info(imp_dir):
    """Get IMP metadata as dict."""
    result = _run(["info", str(imp_dir), "--json"])
    return json.loads(result.stdout)


def loudness(audio_path):
    """Measure audio loudness (EBU R128)."""
    result = _run(["loudness", str(audio_path)])
    return result.stdout


def compliance(imp_dir, standard="netflix"):
    """Check platform compliance."""
    result = _run(["compliance", "-i", str(imp_dir), "-s", standard], check=False)
    return {"compliant": result.returncode == 0, "output": result.stdout}


def encode(input_dir, output_dir, bandwidth=250):
    """Encode image sequence to JPEG 2000."""
    _run(["encode", "--input", str(input_dir), "--output", str(output_dir),
          "--bandwidth", str(bandwidth)])
    return Path(output_dir)


def supplement(ov_dir, title, output_dir, video=None):
    """Create a supplemental IMP."""
    args = ["supplement", "--ov", str(ov_dir), "-t", title, "-o", str(output_dir)]
    if video:
        args.extend(["-v", str(video)])
    _run(args)
    return Path(output_dir)

