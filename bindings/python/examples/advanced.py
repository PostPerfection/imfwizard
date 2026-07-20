#!/usr/bin/env python3
"""Example: validate, transcode, encode, loudness, and info via the CLI."""

import imfwizard


def validate_example():
    report = imfwizard.validate("/path/to/imp", xsd=True)
    print("Valid" if report["valid"] else "Invalid")
    print(report["output"])


def transcode_example():
    """Transcode a source file to another codec via ffmpeg."""
    out = imfwizard.transcode(
        "/path/to/movie.mov", "/tmp/proxy.mov", codec="libx264"
    )
    print(f"Transcoded: {out}")


def encode_example():
    """Encode an image sequence to JPEG 2000 at a target bitrate."""
    out = imfwizard.encode("/tmp/tiff_sequence", "/tmp/j2k_output", bitrate=250)
    print(f"Encoded to: {out}")


def loudness_example():
    """Measure audio loudness (EBU R128). The CLI reports; it does not normalize."""
    print(imfwizard.loudness("/path/to/audio.wav"))


def info_example():
    print(imfwizard.info("/path/to/imp"))


if __name__ == "__main__":
    info_example()
    validate_example()
    transcode_example()
    encode_example()
    loudness_example()
