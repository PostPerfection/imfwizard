# Changelog

## Unreleased

### Added
- **Audio delay**: `create --audio-delay <MILLISECONDS>` shifts the sound against the picture, positive later and negative earlier, spelled as in dcpwizard. The running time never changes: a positive delay prepends silence and truncates the same amount off the tail, a negative one drops the head and pads the tail. A delay at least as long as the programme is refused, naming both durations. The Properties panel carries a Delay (ms) field.
- **Source colour space**: `create --source-colourspace <rec709|p3|xyz|rec2020|aces|acescg|logc>`, defaulting to rec709. The value set is postkit's `ColourSpace`. `rec709` reaches the encoder as the `SourceColour::DisplayRgb` the pipeline already used, so today's output is unchanged; `xyz` says the source is already X'Y'Z' and skips every colour transform. The other five have no postkit transform to X'Y'Z' and are refused rather than run through the Rec.709 matrix, which would be silently wrong colour. `--hdr` together with a source the encoder would transform is refused, since the HDR label claims essence nothing rewrote. The dead `prop-colour` select in the GUI is replaced by a real Source Color Space control.
- **Trim**: `create --trim-start <DURATION>` and `--trim-end <DURATION>`, as frames (`48f`) or seconds (`2s`). Picture, sound and timed text move together: cues shift by the head trim, a cue wholly outside the kept range is dropped and one straddling an edge is clamped. Both TTML time shapes are read, clock times (`00:00:01.500`, `00:00:01:12`) and offset times (`0.8s`, `900ms`, `24f`), and each cue is written back in the metric it arrived in. A time that cannot be read, or timing on anything but a `<p>`, stops the trim rather than being moved wrongly. A trim that leaves nothing is refused, naming the source length. The Properties panel carries a Trim fieldset.
- **Still image with duration**: `create --still-length <DURATION>` holds a single image file (dpx, tif, exr, png or bmp) for that long. The image is encoded once and the codestream repeated, so an hour-long slate costs one encode. `--still-length` without a still input and a still input without `--still-length` are both refused. The Properties panel carries a Still Length field.

### Changed
- **`--video` pointing at one image file now means a still**: it used to encode every image sitting beside it in that folder, which nothing documented. It now needs `--still-length` and encodes only that image. Pointing at the folder still encodes the sequence, unchanged.

### Fixed
- **`to-dcp` wrote a CPL missing its identity fields** — the generated CPL carried no `AnnotationText`, the PKL's was empty, and no reel asset had a `<Hash>`, all of which Bv2.1 wants and libdcp writes. The asset hashes were already computed for the PKL, so the CPL now repeats them
- **KDMs and signatures carried DER-order distinguished names** — the vendored postkit predated the RFC 4514 ordering fix, so a projector matching a KDM recipient saw a name it did not recognise

## [1.2.0] - 2026-08-13

### Added
- **Rebindable GUI keyboard shortcuts**: every shortcut now lives in one registry. Ctrl+K opens an overlay listing them by category, where clicking a shortcut captures a new key combination (Backspace or Delete unbinds, Escape cancels, a combination already in use is refused and names its owner). Per-shortcut and global reset buttons restore the defaults, and changes persist across restarts
- **HDR/WCG essence metadata (ST 2067-21)** — `create --hdr pq-bt2020|pq-p3d65` writes the transfer characteristic + colour primaries ULs onto the picture MXF RGBA essence descriptor (asdcplib `open_write_hdr`) and emits a matching CPL `EssenceDescriptor` linked via `SourceEncoding`. Optional `--mastering-display` (x265 master-display string, requires `--hdr`) adds the ST 2086 block (display primaries, white point, max/min luminance). The CPL claims only what the MXF carries; round-tripped via `hdr_metadata()` and XSD-validated against imf-cpl-20160411.xsd. `--max-cll`/`--max-fall` (nits, also requiring `--hdr`) write the content light levels as ST 2067-21 CPL `ExtensionProperties`, where that standard puts them rather than in the MXF descriptor.
- **Loudness adjustment** — `loudness --adjust-to <lufs> -o <out.wav>` applies a pure sample-domain gain to hit a target integrated loudness (EBU R128), clip-safe: it refuses and writes nothing if the gain would raise true peak above the ceiling (`--true-peak`, default -1 dBTP), reporting the headroom (dom#1382, via postkit `adjust_loudness`).
- **More subtitle input formats** — `subtitle-convert` now reads ASS/SSA (dom#1462), FCPXML (dom#2909), and MKS/Matroska (dom#3131) in addition to SRT and SCC. ASS/FCPXML/MKS keep styling and placement (italic/bold/underline/colour, alignment, vertical position) as IMSC regions and inline `tts:` styling in the TTML output; SRT/SCC and plain targets stay text-only.
- **Accessibility audio (AD/HI)**: `create --audio-role ad|hi` emits an MCA `EssenceDescriptor` (SoundfieldGroup + `chVIN`/`chHI` channel label + RFC 5646 spoken language) linked to the audio resource via `SourceEncoding` (ST 2067-2/-3), carried verbatim by `postkit::packaging::ImfCpl`. XSD-validated against imf-cpl-20160411.xsd.
- **KDM generation** — `kdm` command now generates signed SMPTE 430-1 KDMs via postkit (xmlsec1-verified). New `--signer-cert`/`--signer-key`/`--signer-chain` flags. `--formulation` is now applied rather than ignored: it decides whether the KDM carries a ContentAuthenticator and whether the authorized device list names devices or the assume-trust marker, and an unknown spelling is refused. New repeatable `--device-cert` supplies the devices the two device-listing formulations need. New `-p`/`--disable-forensic-marking-picture` and `-a`/`--disable-forensic-marking-audio [CHANNEL]` write the ST 430-1 ForensicMarkFlagList with DCP-o-matic's semantics: bare `-a` disables every audio channel, a number disables only the channels above it, and omitting either leaves marking on. New `--format smpte|interop` and `--annotation`, spelled as in dcpwizard; Interop output has not been checked against real legacy gear. The command now warns that its KDM carries a freshly minted content key and unlocks no existing essence, since there is still no way to supply real ones.
- **Subtitle packaging** — `create --subtitle` wraps TTML/IMSC files as AS-02 timed-text MXF and adds a SubtitlesSequence to the CPL.
- **GUI metadata save** — the Metadata panel Save button now calls `metadata-edit` instead of showing a placeholder.
- **XML-DSIG signing** — `sign`/`verify-sig` implement standard enveloped signatures via postkit (needs a cert + key).
- **IMF-to-DCP** — `to-dcp` rewraps a single-composition IMP (one picture, optional one sound) to a DCP; multi-reel errors.
- **REST job executor** — `serve` now runs a background worker that executes submitted encode/transcode/validate/loudness/create jobs (in-memory, process-lifetime only). Added `serve --api-key`.
- **GUI audio/subtitle/bandwidth** — GUI builds now wrap the selected audio and subtitle into the IMP and convert the bandwidth setting to a J2K compression ratio for the encode. Previously all three were dropped.
- **VMAF** — `compare --vmaf` computes VMAF via ffmpeg's libvmaf filter (in `--json` too); errors clearly if the local ffmpeg has no libvmaf.
- **Photon validation** — `validate --photon` (off by default) shells out to Netflix Photon (`--photon-jar` or `PHOTON_JAR`) and merges its per-file errors/warnings; errors with an install hint if java or the jar is missing.
- **Supplemental IMP** — `supplement` builds a real ST 2067-2/-3 OV+supplemental package: `--replace`/`--add <path>@<track>` wraps only the new/changed track files, and the CPL references the OV's unchanged track files by UUID (ASSETMAP/PKL cover only present assets). Fails loud when nothing changes or a replace target is absent from the OV.

### Changed
- **Shared GUI frontend via guikit**: the preview and keyboard-shortcut modules and the base stylesheet now come from the `extern/guikit` submodule instead of local copies; `gui/src/style.css` keeps only the imfwizard-specific rules. No behaviour change.
- **J2K encoding is Grok-only**: dropped the `openjpeg` postkit feature; the CLI `create` video path now encodes through the shared `postkit::pipeline` grok (`grk_compress`) pipeline, the same one the GUI uses. No OpenJPEG fallback; errors clearly if `grk_compress` is not found.
- **CPL language via postkit**: the composition LocaleList now comes from `ImfCpl.languages` (postkit emits it) instead of imfwizard splicing the block into the generated XML. Output is byte-identical.
- **IMF-to-DCP writers use postkit**: `to-dcp` now writes its DCP CPL/PKL/ASSETMAP/VOLINDEX through `postkit::packaging` (`DcpCpl` with per-reel picture dims for a real `ScreenAspectRatio`) instead of hand-rolled XML. Schema-valid (dcpdoctor schema-validate: passed).
- **MXF wrapping** — imfwizard-core delegates J2K/TimedText/Atmos wrapping to postkit's AS-02 writers. PCM now parses the real WAV header (channels/bits/sample rate) instead of hardcoding 5.1/24-bit/48k.
- **target-convert** — maps 2k/4k scope/flat/full to real resolutions and errors on unknown targets instead of always producing 1080p.
- **metadata-edit** — the `--issuer` value is now written (was discarded).
- **Watermark** — visible burn-in only via postkit; removed NexGuard/Civolution options and the `--backend` flag.
- **Atmos import** — wraps essence via asdcplib instead of shelling ffmpeg for the MXF; ADM XML parsed with quick-xml.
- **Timeline/ASSETMAP parsing** — replaced hand-rolled line scanners with quick-xml.
- **Packaging XML** — CPL/PKL/ASSETMAP writers and the SRT parser now use the shared `postkit::packaging` / `postkit::subtitle_retime` writers instead of hand-rolled copies.

### Fixed
- **GUI "Show in Files"**: reveals the asset via `tauri-plugin-opener` `revealItemInDir` instead of `plugin:shell open` on a stripped parent path (shell open only accepts URLs). Mirrors the dcpwizard fix.

### Removed
- **`daemon` and `batch` commands** — an in-memory, cross-process queue with no IPC could never run or persist jobs; removed in favour of the real `serve` worker.
- **OpenJPEG**: no longer referenced anywhere in the repo; J2K encoding is Grok-only.

## [1.1.0] — 2026-05-28

### Added
- **Timeline view** — Visual IMF timeline with multi-segment navigation and CPL structure display
- **Batch delivery** — Submit multiple delivery jobs (one per target platform) from the Deliver panel
- **CLI flag consistency test** — `tests/cli_flags_test.sh` verifies GUI invocations match actual CLI flags
- **CI: Rust CLI job** — Full Rust build, test, clippy, fmt, and flag check in GitHub Actions

### Fixed
- **Properties panel → build pipeline** — Framerate, content_kind, and bandwidth now correctly passed through to ImpOptions
- **Validate** — Uses `analyze -i` instead of non-existent `validate` subcommand
- **Analytics** — Uses `analyze -i --json` instead of non-existent `analytics` subcommand
- **Transcode** — Removed non-existent `-f` format flag
- **Metadata browse** — Uses `analyze -i --json` instead of non-existent `info` subcommand
- **Jobs view** — Uses internal Tauri `list_jobs`/`cancel_job` instead of non-existent external batch daemon
- **Metadata save** — Button now wired (shows "not yet supported" message)
- **Target conversion** — Removed `Command.create` (would never find bundled binary)

### Known Limitations
- Loudness measurement and supplemental IMP creation show informational messages (CLI support not yet available). Metadata save, subtitle packaging, and target conversion are wired to the CLI as of Unreleased.

## [1.0.0] — 2025-01-20

### Added
- **CLI: Create subcommand** — Full IMF package creation
- **CLI: Validate subcommand** — IMF package validation
- **CLI: Transcode subcommand** — Media transcoding for IMF workflows
- **Panic hook** — User-friendly crash messages with issue tracker link
- **Release CI** — GitHub Actions workflow for building release binaries on tag push
- **GUI Release CI** — Tauri build workflow producing .deb, .AppImage, .dmg, .msi

### Changed
- Version unified to 0.5.0 across all workspace crates
- Git dependencies pinned to v0.5.0 tags (asdcplib-rs, dcpdoctor, postkit)

### Fixed
- Clippy warnings cleaned up across entire workspace
