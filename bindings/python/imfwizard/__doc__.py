"""IMF Wizard Python Bindings

A thin subprocess wrapper around the `imfwizard` CLI for creating SMPTE ST 2067
IMF packages. There is no compiled extension: every function shells out to the
`imfwizard` binary (found on PATH or via the IMFWIZARD_BIN environment variable).

Functions:
    create(title, video, output, audio=None, subtitle=None, kind="feature")
        Build an IMP from a video file or J2K directory. Returns the output Path.
    validate(imp_dir, xsd=False) -> {"valid": bool, "output": str}
    info(imp_dir) -> str
    loudness(audio_path) -> str
    compliance(imp_dir, standard="smpte") -> {"compliant": bool, "output": str}
    encode(input_dir, output_dir, bitrate=250) -> Path
    transcode(input_file, output_file, codec="libx264") -> Path
    subtitle_convert(input_file, output_ttml) -> Path

Example:
    >>> import imfwizard
    >>> out = imfwizard.create(
    ...     title="My Feature",
    ...     video="/path/to/master.mov",
    ...     output="/path/to/output_imp",
    ...     audio="/path/to/audio.wav",
    ... )
    >>> imfwizard.validate(out)
    {'valid': True, 'output': 'IMP validation PASSED\\n'}
"""
