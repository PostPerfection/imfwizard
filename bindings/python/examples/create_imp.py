#!/usr/bin/env python3
"""Example: create an IMF package from Python."""

import imfwizard


def main():
    out = imfwizard.create(
        title="My Feature Film",
        video="/path/to/master.mov",  # video file or J2K directory
        output="/tmp/my_imp",
        audio="/path/to/audio.wav",
        subtitle="/path/to/subtitles.ttml",  # or a list of paths
        kind="feature",
    )
    print(f"IMP created: {out}")

    report = imfwizard.validate(out)
    print("Valid" if report["valid"] else "Invalid")
    print(report["output"])


if __name__ == "__main__":
    main()
