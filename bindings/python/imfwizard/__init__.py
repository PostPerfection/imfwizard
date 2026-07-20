"""IMF Wizard — Python bindings for creating IMF packages.

Provides a subprocess-based API that wraps the imfwizard CLI.
Requires the `imfwizard` binary to be on PATH or specified via IMFWIZARD_BIN.
"""

import os
import shutil
import subprocess
from pathlib import Path

__version__ = "1.1.0"


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


def create(title, video, output, audio=None, subtitle=None, kind="feature"):
    """Create an IMP from source media.

    `subtitle` may be a single path or a list of paths (each becomes a
    repeated --subtitle). `video` may be a video file or a J2K directory.
    """
    args = ["create", "--title", title, "--video", str(video),
            "--output", str(output), "--kind", kind]
    if audio:
        args.extend(["--audio", str(audio)])
    if subtitle:
        subs = subtitle if isinstance(subtitle, (list, tuple)) else [subtitle]
        for s in subs:
            args.extend(["--subtitle", str(s)])
    _run(args)
    return Path(output)


def validate(imp_dir, xsd=False):
    """Validate an IMP directory. Returns dict with valid/output.

    The CLI prints a text report and exits non-zero on failure; pass
    xsd=True to also check XML against the SMPTE ST 2067 schemas.
    """
    args = ["validate", str(imp_dir)]
    if xsd:
        args.append("--xsd")
    result = _run(args, check=False)
    return {
        "valid": result.returncode == 0,
        "output": result.stdout + result.stderr,
    }


def info(imp_dir):
    """Get IMP metadata as text."""
    return _run(["info", str(imp_dir)]).stdout


def loudness(audio_path):
    """Measure audio loudness (EBU R128). Returns the CLI text report."""
    return _run(["loudness", str(audio_path)]).stdout


def compliance(imp_dir, standard="smpte"):
    """Check platform compliance (smpte, netflix, dolby, amazon)."""
    result = _run(["compliance", "-i", str(imp_dir), "-s", standard], check=False)
    return {"compliant": result.returncode == 0, "output": result.stdout}


def encode(input_dir, output_dir, bitrate=250):
    """Encode an image sequence to JPEG 2000 codestreams."""
    _run(["encode", "-i", str(input_dir), "-o", str(output_dir),
          "-b", str(bitrate)])
    return Path(output_dir)


def transcode(input_file, output_file, codec="libx264"):
    """Transcode media via ffmpeg."""
    _run(["transcode", "-i", str(input_file), "-o", str(output_file),
          "-c", codec])
    return Path(output_file)


def subtitle_convert(input_file, output_ttml):
    """Convert an SRT subtitle file to TTML for IMF."""
    _run(["subtitle-convert", "-i", str(input_file), "-o", str(output_ttml)])
    return Path(output_ttml)
