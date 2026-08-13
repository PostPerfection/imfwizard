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
- encode, transcode (basic), subtitle-convert SRT and SCC (CEA-608 pop-on) to IMSC/TTML, hash, timecode. Authored TTML/IMSC is copied unchanged when converting to TTML, preserving regions, placement, and styling.
- Structural validate (dcpdoctor-core) plus XSD via xmllint; loudness measure; info; doctor. Optional `validate --photon` shells out to Netflix Photon (JRE + jar) and merges its findings.
- kdm, dcdm, colour, lut, aces CTL pipeline, dv-extract/dv-inject/dv-convert, watermark, trailer, restore (asdcp-unwrap).
- `supplement`: real ST 2067-2/-3 OV+supplemental IMP. Reads the OV CPL (edit rate, content kind, per-kind track-file UUIDs), wraps only the new/changed assets (`--replace`/`--add`, `<path>@<track>` selectors), and writes a CPL referencing the OV UUIDs for unchanged tracks plus the new UUIDs for changed ones. ASSETMAP/PKL cover only the physically-present assets (new CPL + new track files). Fails loud when nothing changes or a `--replace` target does not exist in the OV.
- sign/verify-sig: real enveloped XML-DSIG via postkit (needs a cert + key). to-dcp: rewraps a single-composition IMP (one picture, optional one sound) to a DCP; multi-reel errors. It only accepts pre-converted DCI XYZ picture essence and never applies HDR-to-DCP conversion.
- compare: PSNR/SSIM (`--pixel`) plus optional VMAF (`--vmaf`, shells out to ffmpeg's libvmaf filter); JSON output carries both.
- deliver upload (s3/aspera/rsync) with real SQLite tracker; target-convert with real 2k/4k mappings; compliance ffprobe checks; atmos ADM import (PCM); mca CPL injection; EDL/FCP7-xmeml parse; analytics with per-second bitrate histogram.
- REST API (`serve`): a background worker executes submitted encode/transcode/validate/loudness/create jobs through the same core paths as the CLI. The queue is in-memory, so jobs live only for the server process. `serve --api-key` enables auth. Unsupported job types fail loud; no path silently drops work.
- GUI: builds wrap the selected audio and subtitle into the IMP and map the bandwidth setting to a J2K compression ratio; preview player/scrubber, timeline viewer, jobs manager, notifications, recents, theme.

## Deliberately not implemented (fails loud, de-advertised)

- `daemon` / `batch` CLI commands: removed. An in-memory cross-process queue with no IPC could never run or persist jobs; use `serve` (real worker) or the direct per-operation commands.
- Photon and VMAF are optional shell-out integrations (off by default, no build deps), not core features.
- HDR/WCG (ST 2067-21): `create --hdr pq-bt2020|pq-p3d65` writes the transfer/colour ULs on the picture MXF descriptor (asdcplib `open_write_hdr`) and emits the matching CPL RGBADescriptor EssenceDescriptor + SourceEncoding; `--mastering-display <x265 string>` adds the ST 2086 block; `--max-cll`/`--max-fall` (u16 nits, refused without `--hdr`) land in the CPL ExtensionProperties beside ApplicationIdentification as app2e `xs:unsignedShort` elements, byte-identical output when absent. ST 2067-21 carries the light levels in the CPL, not the MXF descriptor, SMPTE defined no descriptor membership for them (the registered ULs belong to an ST 2108-2 serial-interface pack), so asdcplib rightly has no property. Photon schema-validates the pair and never compares them to the essence. The XSD tests import app2e-2016.xsd, since ExtensionProperties is `xs:any` lax and xmllint would otherwise skip the elements.
- Accessibility audio (`create --audio-role ad|hi`): emits an MCA EssenceDescriptor (WAVEPCMDescriptor + SoundfieldGroup + chVIN/chHI AudioChannelLabel + RFC5646SpokenLanguage) linked to the audio resource via SourceEncoding. Audio language is also expressed at the composition level (LocaleList). SL stays a video overlay with no MCA audio descriptor (see DESIGN_TODO).
- SCC conversion handles pop-on captions only; roll-up, paint-on, and text-mode fail loud.

## Layout note

imfwizard builds against the `extern/postkit` submodule (pinned to the postkit commit with the `postkit::packaging` dedup work); bump the pin when postkit changes. dcpdoctor-core is pinned at rev ce050e5.
