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
- encode, transcode (basic), subtitle-convert SRT and SCC (CEA-608 pop-on) to IMSC/TTML, hash, timecode. Authored TTML/IMSC is copied unchanged when converting to TTML, preserving regions, placement, and styling. `--font-size` and `--colour` write a `tts:fontSize`/`tts:color` style that every `<p>` points at, which per-run styling overrides, and are refused against an authored TTML input since the copy would drop them. The size goes out as a cell-relative length against a declared `ttp:cellResolution`, because a bare percentage in TTML is read against the parent element's own size rather than the frame.
- Structural validate (dcpdoctor-core) plus XSD via xmllint; loudness measure; info; doctor. Optional `validate --photon` shells out to Netflix Photon (JRE + jar) and merges its findings.
- kdm, dcdm, colour, lut, aces CTL pipeline, dv-extract/dv-inject/dv-convert, watermark, trailer, restore (asdcp-unwrap).
- `supplement`: real ST 2067-2/-3 OV+supplemental IMP. Reads the OV CPL (edit rate, content kind, per-kind track-file UUIDs), wraps only the new/changed assets (`--replace`/`--add`, `<path>@<track>` selectors), and writes a CPL referencing the OV UUIDs for unchanged tracks plus the new UUIDs for changed ones. ASSETMAP/PKL cover only the physically-present assets (new CPL + new track files). Fails loud when nothing changes or a `--replace` target does not exist in the OV.
- sign/verify-sig: real enveloped XML-DSIG via postkit (needs a cert + key). to-dcp: rewraps a single-composition IMP (one picture, optional one sound) to a DCP; multi-reel errors. It only accepts pre-converted DCI XYZ picture essence and never applies HDR-to-DCP conversion.
- compare: PSNR/SSIM (`--pixel`) plus optional VMAF (`--vmaf`, shells out to ffmpeg's libvmaf filter); JSON output carries both.
- deliver upload (s3/aspera/rsync) with real SQLite tracker; target-convert with real 2k/4k mappings; compliance ffprobe checks; atmos ADM import (PCM); mca CPL injection; EDL/FCP7-xmeml parse; analytics with per-second bitrate histogram.
- REST API (`serve`): a background worker executes submitted encode/transcode/validate/loudness/create jobs through the same core paths as the CLI. The queue is in-memory, so jobs live only for the server process. `serve --api-key` enables auth. Unsupported job types fail loud; no path silently drops work.
- Burn-in during the encode (`subtitle_burn.rs`): `create --burn-subtitle` parses SRT/ASS/SCC/FCPXML/MKS to `StyledCue`s and hands them to postkit's `SubtitleBurn`, which composites them onto every decoded frame before the colour transform. Nothing is registered in the CPL, since the text is picture. `check_burn_supported` refuses the combinations that would draw in the wrong place before an encode starts: the same file as `--subtitle`, a J2K directory, and any route that hands the encoder X'Y'Z' already (`--source-colourspace xyz` or `--hdr`). Appearance comes from postkit's `BurnStyleOverrides`, laid over `BurnStyle::default()` by `resolve_burn_style`: `--burn-font-size`, `--burn-colour`, `--burn-effect`, `--burn-effect-colour`, `--burn-outline-width`, `--burn-x-scale`, `--burn-y-scale`, `--burn-fade-up` and `--burn-fade-down`, each left unset unless the caller names it. Colours parse with `Rgba::parse_hex` and effects with `parse_burn_effect`, and the ranges are postkit's, so nothing here reimplements either. The GUI Properties panel carries the same fields bar the two scales, which are CLI-only, and runs the same checks in `submit_job`. A held still never reaches `postkit::pipeline`: `still.rs` decodes the image with ffmpeg and drives `grok_encoder::encode_pipeline` itself, encoding one frame per run of frames sharing a cue set and linking within the run, so a burnt hold costs a handful of encodes.
- Source picture processing (`source_picture.rs`): `create` takes per-side crop, `--auto-crop` (postkit `detect_black_borders` over eight seeked samples, unioned), `--fill-crop`, `--deinterlace`, `--denoise`, `--rotate`, `--flip` and `--raster`, and resolves them into a postkit `PictureProcessing` plan that the encode applies while ffmpeg decodes. The three crops decide the same thing, so only one may be given. The fit rule: the picture is fitted into `--raster` when one is named and into the source raster otherwise, and a `Fit` is set whenever anything is not the identity or the target differs from the source, so the encode raster is always either an App 2E raster or the source untouched. `validate_app2e_raster` therefore runs on the plan's output rather than on the probed source, in the CLI and the GUI alike, and both log `PicturePlan::describe()`. A J2K directory is refused before the encode, since it never decodes. A held still runs the same filters inside `still.rs`'s own ffmpeg decode, so the crop and the raster fit reach it too.
- Audio mix matrix (`audio_map.rs`): `create --audio-map "1:L,2:R,1:C@-6"` parses postkit's `IN:OUT[@GAIN]` grammar with the destination allowed to be a channel name as `channel_map.rs` spells it (the label or the MCA symbol with or without its `ch` prefix, case-insensitive), and the highest destination lane sets the output channel count. The map runs on the `--audio` WAV before the delay, the trim and the MCA labelling, so the labelled layout describes the file that is packaged. `--audio-map` without `--audio` is refused: the track demuxed from `--video` is written after the map would have run. The GUI's Audio section carries the same map as a matrix of dB cells, serialised to the same spec string.
- GUI: builds wrap the selected audio and subtitle into the IMP and map the bandwidth setting to a J2K compression ratio; preview player/scrubber, timeline viewer, jobs manager, notifications, recents, theme.

## Deliberately not implemented (fails loud, de-advertised)

- `daemon` / `batch` CLI commands: removed. An in-memory cross-process queue with no IPC could never run or persist jobs; use `serve` (real worker) or the direct per-operation commands.
- Photon and VMAF are optional shell-out integrations (off by default, no build deps), not core features.
- HDR/WCG (ST 2067-21): `create --hdr pq-bt2020|pq-p3d65` writes the transfer/colour ULs on the picture MXF descriptor (asdcplib `open_write_hdr`) and emits the matching CPL RGBADescriptor EssenceDescriptor + SourceEncoding; `--mastering-display <x265 string>` adds the ST 2086 block; `--max-cll`/`--max-fall` (u16 nits, refused without `--hdr`) land in the CPL ExtensionProperties beside ApplicationIdentification as app2e `xs:unsignedShort` elements, byte-identical output when absent. ST 2067-21 carries the light levels in the CPL, not the MXF descriptor, SMPTE defined no descriptor membership for them (the registered ULs belong to an ST 2108-2 serial-interface pack), so asdcplib rightly has no property. Photon schema-validates the pair and never compares them to the essence. The XSD tests import app2e-2016.xsd, since ExtensionProperties is `xs:any` lax and xmllint would otherwise skip the elements.
- Accessibility audio (`create --audio-role ad|hi`): emits an MCA EssenceDescriptor (WAVEPCMDescriptor + SoundfieldGroup + chVIN/chHI AudioChannelLabel + RFC5646SpokenLanguage) linked to the audio resource via SourceEncoding. Audio language is also expressed at the composition level (LocaleList). SL stays a video overlay with no MCA audio descriptor (see DESIGN_TODO).
- SCC conversion handles pop-on captions only; roll-up, paint-on, and text-mode fail loud.
- Source colour space (`create --source-colourspace`): postkit decides the encoder
  transform from `encode::SourceColour`, which offers three shapes, so only two of
  the seven `ColourSpace` values map. `rec709` is `SourceColour::DisplayRgb`, the
  variant `EncodeRunOptions::default()` already used, which is what keeps the
  default output unchanged. `xyz` is `SourceColour::AlreadyPq`; that name is about
  where postkit met the case (HDR essence), and skipping the transform is its only
  effect, which is exactly what an already-X'Y'Z' source needs. The other five have
  no postkit transform to X'Y'Z' at all: `convert_colour` refuses them without a 3D
  LUT and `rgb_to_xyz_inplace` is Rec.709-only, so they are refused rather than run
  through the Rec.709 matrix (see DESIGN_TODO). `--hdr` plus a source colour space
  the encoder would transform is refused, because postkit's own rule is that
  essence a caller labels PQ can never hold frames the encoder rewrote; `--hdr`
  with `xyz` composes and `--hdr` alone is unchanged. `xyz` reaches only the video
  input: postkit's image-sequence encoder always applies the transform and refuses
  any other source colour, so an image sequence or a still stays rec709-only, and
  a J2K directory reaches the wrapper with no encode at all, so anything other
  than rec709 is refused there rather than dropped in silence. The GUI select
  offers only the two values that work; the CLI takes all seven spellings so the
  five without a transform fail with an explanation, not an unknown-value error.
- Timed text trim (`source_edits.rs`) reads both TTML time shapes, clock times
  (`00:00:01.500`, `00:00:01:12`) and offset times (`0.8s`, `900ms`, `24f`), and
  writes each cue back in the metric it arrived in. A time it cannot read, ticks
  included, stops the trim rather than resolving to zero, which would silently
  drop the cue. The frame field is rendered from a whole-frame count, since
  computing it as a fraction of a second lets rounding name frame 24 at 24 fps.
  Only `<p>` may carry timing: TTML times a timed element's children relative to
  it, so timing on a `<div>` or `<span>` is refused rather than shifted, which
  would move its cues twice.
- Every staging directory (`j2k_trimmed`, `j2k_still`, the still's own source and
  encode dirs) is emptied before it is written. The MXF wrapper takes the whole
  directory listing, so building into the same output folder again with a shorter
  trim or still length would otherwise package the longer first run.
- Source edits ordering (`source_edits.rs`): the audio delay lands before the trim.
  The delay says how sound lines up with picture; the trim then cuts a range out of
  the aligned programme. The other order would cut a range that had not been
  aligned yet, so the same flags would give a different result.
- Trim runs after the encode, not before it: `postkit::pipeline` has no hook for a
  frame range, so the trimmed picture is a directory of hard links to the kept
  codestreams. That costs no disk but does encode frames it then discards.
- A still (`create --still-length`) is staged alone in a directory so the
  image-sequence encoder sees one frame rather than everything beside it, encoded
  once, then hard-linked once per held frame. Its ffmpeg decode carries the
  picture plan's filters and is sized by the plan's output, which is how a still
  meets the same crop, turn and raster fit a video meets inside the pipeline. Both paths write `frame_%08d.j2c`,
  since the MXF wrapper takes the sorted directory listing as playing order.

## Layout note

imfwizard builds against the `extern/postkit` submodule (pinned to the postkit commit with the `postkit::packaging` dedup work); bump the pin when postkit changes. dcpdoctor-core is pinned at rev ce050e5.

The GUI frontend takes its preview module, keyboard shortcut engine and base stylesheet from the `extern/guikit` submodule, shared with dcpwizard; `gui/src/style.css` holds only the imfwizard deltas. Bump the pin when guikit changes.
