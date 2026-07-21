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

- IMP creation from J2K/video + WAV + TTML via CLI: real Grok (`grk_compress`) encode, AS-02 wrapping, CPL/PKL/ASSETMAP with base64 SHA-1, App2E precheck. The CPL/PKL/ASSETMAP writers and SRT parsing come from `postkit::packaging` / `postkit::subtitle_retime` (shared with dcpwizard). `--audio-lang` writes an RFC 5646 LocaleList/Language in the CPL (ST 2067-3, XSD-validated); `--audio-role ad|hi` adds an MCA accessibility EssenceDescriptor linked by SourceEncoding; `--profile <preset>` maps a delivery preset's bitrate to the J2K ratio.
- Multi-CPL IMP: `create_imp` takes a list of compositions and writes one CPL each over a single shared PKL/ASSETMAP. The CLI writes one composition; the GUI packages every composition tab into one multi-CPL IMP.
- encode, transcode (basic), subtitle-convert SRT and SCC (CEA-608 pop-on) to IMSC/TTML, hash, timecode.
- Structural validate (dcpdoctor-core) plus XSD via xmllint; loudness measure; info; doctor. Optional `validate --photon` shells out to Netflix Photon (JRE + jar) and merges its findings.
- kdm, dcdm, colour, lut, aces CTL pipeline, dv-extract/dv-inject/dv-convert, watermark, trailer, restore (asdcp-unwrap).
- `supplement`: real ST 2067-2/-3 OV+supplemental IMP. Reads the OV CPL (edit rate, content kind, per-kind track-file UUIDs), wraps only the new/changed assets (`--replace`/`--add`, `<path>@<track>` selectors), and writes a CPL referencing the OV UUIDs for unchanged tracks plus the new UUIDs for changed ones. ASSETMAP/PKL cover only the physically-present assets (new CPL + new track files). Fails loud when nothing changes or a `--replace` target does not exist in the OV.
- sign/verify-sig: real enveloped XML-DSIG via postkit (needs a cert + key). to-dcp: rewraps a single-composition IMP (one picture, optional one sound) to a DCP; multi-reel errors.
- compare: PSNR/SSIM (`--pixel`) plus optional VMAF (`--vmaf`, shells out to ffmpeg's libvmaf filter); JSON output carries both.
- deliver upload (s3/aspera/rsync) with real SQLite tracker; target-convert with real 2k/4k mappings; compliance ffprobe checks; atmos ADM import (PCM); mca CPL injection; EDL/FCP7-xmeml parse; analytics with per-second bitrate histogram.
- REST API (`serve`): a background worker executes submitted encode/transcode/validate/loudness/create jobs through the same core paths as the CLI. The queue is in-memory, so jobs live only for the server process. `serve --api-key` enables auth. Unsupported job types fail loud; no path silently drops work.
- GUI: builds wrap the selected audio and subtitle into the IMP and map the bandwidth setting to a J2K compression ratio; preview player/scrubber, timeline viewer, jobs manager, notifications, recents, theme.

## Deliberately not implemented (fails loud, de-advertised)

- `daemon` / `batch` CLI commands: removed. An in-memory cross-process queue with no IPC could never run or persist jobs; use `serve` (real worker) or the direct per-operation commands.
- Photon and VMAF are optional shell-out integrations (off by default, no build deps), not core features.
- Accessibility roles (AD/HI/SL) and HDR/WCG (ST 2067-21) essence metadata have no create-path option: both need per-track MCA labels / an EssenceDescriptorList that `postkit::packaging::ImfCpl` does not emit. Audio language is expressed at the composition level (LocaleList) instead.
- SCC conversion handles pop-on captions only; roll-up, paint-on, and text-mode fail loud.

## Layout note

imfwizard builds against the `extern/postkit` submodule (pinned to the postkit commit with the `postkit::packaging` dedup work); bump the pin when postkit changes. dcpdoctor-core is pinned at rev ce050e5.
