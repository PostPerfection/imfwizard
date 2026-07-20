# Design

IMF package creation tool. Rust core with CLI, Tauri GUI, and python bindings.

## Layout

- `rust/crates/imfwizard-core`: IMP assembly (CPL/PKL/ASSETMAP writers, AS-02 wrap via postkit/asdcplib), validation, delivery, REST API.
- `rust/crates/imfwizard-cli`: clap CLI (create, encode, transcode, validate, kdm, deliver, compliance, analytics, ...).
- `gui/`: Tauri app, CLI as sidecar. Largely shares code shape with dcpwizard's gui (copy, not a package).
- `bindings/python`: subprocess wrapper around the CLI.
- Shares code via postkit (path dep), asdcplib-rs (git), and dcpdoctor-core (git) for validation.
- Many core modules are thin `pub use postkit::...` re-export shims.

## What is implemented and wired

- IMP creation from J2K/video + WAV + TTML via CLI: real OpenJPEG encode, AS-02 wrapping, CPL/PKL/ASSETMAP with base64 SHA-1, App2E precheck. The CPL/PKL/ASSETMAP writers and SRT parsing come from `postkit::packaging` / `postkit::subtitle_retime` (shared with dcpwizard).
- encode, transcode (basic), subtitle-convert SRT to TTML, hash, timecode.
- Structural validate (dcpdoctor-core) plus XSD via xmllint; loudness measure; info; doctor. Optional `validate --photon` shells out to Netflix Photon (JRE + jar) and merges its findings.
- kdm, dcdm, colour, lut, aces CTL pipeline, dv-extract/dv-inject/dv-convert, watermark, trailer, restore (asdcp-unwrap).
- sign/verify-sig: real enveloped XML-DSIG via postkit (needs a cert + key). to-dcp: rewraps a single-composition IMP (one picture, optional one sound) to a DCP; multi-reel errors.
- compare: PSNR/SSIM (`--pixel`) plus optional VMAF (`--vmaf`, shells out to ffmpeg's libvmaf filter); JSON output carries both.
- deliver upload (s3/aspera/rsync) with real SQLite tracker; target-convert with real 2k/4k mappings; compliance ffprobe checks; atmos ADM import (PCM); mca CPL injection; EDL/FCP7-xmeml parse; analytics with per-second bitrate histogram.
- REST API (`serve`): a background worker executes submitted encode/transcode/validate/loudness/create jobs through the same core paths as the CLI. The queue is in-memory, so jobs live only for the server process. `serve --api-key` enables auth. Unsupported job types fail loud; no path silently drops work.
- GUI: builds wrap the selected audio and subtitle into the IMP and map the bandwidth setting to a J2K compression ratio; preview player/scrubber, timeline viewer, jobs manager, notifications, recents, theme.

## Deliberately not implemented (fails loud, de-advertised)

- `supplement`: errors instead of building a mislabelled standalone IMP.
- `daemon` / `batch` CLI commands: removed. An in-memory cross-process queue with no IPC could never run or persist jobs; use `serve` (real worker) or the direct per-operation commands.
- `webhook`, `plugin`, `otioz_import` modules are unused. Photon and VMAF are optional shell-out integrations (off by default, no build deps), not core features.

## Layout note

imfwizard builds against the shared `../../postkit` checkout (not the stale `extern/postkit` submodule) so the `postkit::packaging` dedup work is available. dcpdoctor-core is pinned at rev ce050e5.
