#!/usr/bin/env python3
"""Example: multi-platform delivery and subtitle conversion via the CLI."""

import imfwizard


def batch_workflow():
    """Create an IMP per platform and check each against its compliance profile.

    There is no single batch-deliver command; this loops the real `create` and
    `compliance` operations.
    """
    for target in ("netflix", "dolby", "amazon", "smpte"):
        out = imfwizard.create(
            title=f"My Film ({target})",
            video="/path/to/master.mov",
            output=f"/tmp/deliveries/{target}",
        )
        report = imfwizard.compliance(out, standard=target)
        status = "compliant" if report["compliant"] else "NON-compliant"
        print(f"{target}: {out} — {status}")


def subtitle_workflow():
    """Convert an SRT file to TTML for packaging."""
    ttml = imfwizard.subtitle_convert("/path/to/subs.srt", "/tmp/subs.ttml")
    print(f"TTML written: {ttml}")


if __name__ == "__main__":
    batch_workflow()
    subtitle_workflow()
