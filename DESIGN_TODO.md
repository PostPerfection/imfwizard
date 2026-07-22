# Planned

Paths: CORE = rust/crates/imfwizard-core/src, CLI = rust/crates/imfwizard-cli/src/main.rs.

## DoM tracker gaps (2026-07-22)

The DCP-o-matic Mantis sweep (dom#N = https://dcpomatic.com/bugs/view.php?id=N)
is mostly DCP-side; the items that map here:

- Loudness adjustment to a target (dom#1382): loudness measures only. Gate on the
  postkit gain API (postkit DESIGN_TODO, same date).
- More subtitle input formats via postkit parsers: FCPXML (dom#2909), ASS with
  styling (dom#1462), MKS (dom#3131).
- Export a composition to an image file sequence (dom#3021).

## Fixed 2026-07-20 (silent data loss + doc lies)

- GUI builds no longer drop audio/subtitles/bandwidth: `gui/src-tauri/src/pipeline.rs`
  wires the selected audio and subtitle into `ImpOptions` and converts the
  bandwidth (Mbps) to a J2K compression ratio (via `postkit::pipeline::run_encode_with_ratio`).
- REST API executes jobs: `executor.rs` runs a worker over the queue through the
  same core paths as the CLI (encode/transcode/validate/loudness/create); other
  types fail loud. Wired into `serve`, which also gained `--api-key`. Queue is
  in-memory (documented).
- `daemon` and `batch` CLI commands removed (unrunnable in-memory cross-process queue).
- `supplement` implements the real ST 2067-2/-3 OV+supplemental IMP (`supplement.rs`):
  parses the OV CPL, wraps only new/changed track files (`--replace`/`--add`
  `<path>@<track>` selectors), writes a CPL referencing the OV's unchanged UUIDs
  plus the new ones, and ASSETMAP/PKL over only present assets. Fails loud when
  nothing changes or a replace target is absent from the OV. Was previously de-advertised.
- Python bindings call real CLI flags; `__doc__.py` and all three examples rewritten
  to the actual subprocess API (the fictional SWIG API is gone).
- Docs (README, docs/index.html, CHANGELOG, Dockerfile): removed daemon/batch/IAB/AV1/
  normalization/segment-replacement claims; fixed prores (encodes TO ProRes), rest-api
  flags (--bind/--api-key), and the compare/mca/audio-desc/av-sync/annotate/partial-version/
  slate/transcode/analytics examples to match real flags. Signing and single-composition
  to-dcp are documented as implemented (they are).
- VMAF and Photon implemented as optional shell-out integrations: `compare --vmaf` (ffmpeg
  libvmaf, frame_compare.rs) and `validate --photon [--photon-jar]` (JRE + Photon jar,
  photon.rs). Off by default; both error with a one-line hint when the tool is missing.
- Dedup onto postkit: cpl.rs/pkl.rs/assetmap.rs use `postkit::packaging`
  (ImfCpl/PackingList/AssetMap); subtitle_convert.rs uses `postkit::subtitle_retime::parse_srt`;
  to_dcp.rs uses `postkit::packaging::escape_xml`. dcpdoctor-core bumped to ce050e5.
  imfwizard builds against the `extern/postkit` submodule; bump the pin when postkit changes.

## Fixed 2026-07-21 (overstated/unimplemented items)

- Dead modules deleted (zero callers): webhook, plugin, otioz_import, imp_diff,
  subtitle_retime shim. captions.rs (unused extract_captions) deleted too.
- Multi-CPL IMP: `create_imp` takes `Vec<Composition>`, writes one CPL each over a
  single shared PKL/ASSETMAP (`imp.rs`/`cpl.rs`/`pkl.rs`/`assetmap.rs`). GUI Build
  submits every composition tab (`pipeline.rs` + `main.js`, `submit_job` now takes a
  `compositions` array). CLI still writes one composition.
- RFC 5646 audio language: `create --audio-lang <tag>` writes a composition-level
  LocaleList/Language in the CPL (ST 2067-3). Validated against imf-cpl-20160411.xsd
  via xmllint (gated test `cpl::language_cpl_passes_st2067_3_xsd`).
- SCC (CEA-608) pop-on to IMSC/TTML: new `scc.rs` parser wired into subtitle-convert;
  roll-up/paint-on/text-mode fail loud.
- Authored TTML/IMSC now passes through `subtitle-convert` unchanged when the target
  is TTML, preserving regions, placement, and styling. Non-SCC text formats still
  convert as plain timed text.
- Delivery presets applied: `create --profile <preset>` maps the preset bitrate to the
  J2K compression ratio (`profiles::platform_from_name` + `profile_for`).
- av-sync now compares per-stream start and end on the container clock (initial offset
  plus drift over the program), not just the first PTS.

## Fixed 2026-07-21 (postkit fd477a5 API adoption)

- postkit pin bumped to fd477a5. `ImfResource` gained `source_encoding`, `ImfCpl`
  gained `languages` + `essence_descriptors`; all call sites updated (cpl.rs,
  supplement.rs). cpl.rs dropped its `inject_locale_list` string-splice and now
  passes `ImfCpl.languages` (byte-identical LocaleList; existing tests unchanged).
- Accessibility audio (AD/HI): `create --audio-role ad|hi` (imp.rs `AudioRole`)
  emits an MCA `EssenceDescriptor` (WAVEPCMDescriptor + SoundfieldGroup + chVIN/chHI
  AudioChannelLabel + RFC5646SpokenLanguage) linked to the audio resource via
  SourceEncoding. Body matches the shape in postkit's own XSD-gated packaging test.
  New gated test `cpl::accessibility_cpl_passes_st2067_3_xsd` passes xmllint against
  imf-cpl-20160411.xsd. SL (sign language) is a video overlay, not an audio essence,
  so it gets no MCA audio descriptor here.
- to_dcp.rs: hand-rolled DCP CPL/PKL/ASSETMAP/VOLINDEX writers deleted; now uses
  `postkit::packaging` (`DcpCpl` per-reel picture dims → real ScreenAspectRatio).
  Gated test `to_dcp::generated_dcp_docs_pass_smpte_xsd` + `dcpdoctor schema-validate`
  both pass on the output.
- openjpeg removed: dropped the postkit `openjpeg` feature and the direct
  `openjpeg_encoder` call; CLI `create` now encodes through `postkit::pipeline`
  (grok / grk_compress), the same path the GUI uses. No fallback.
- GUI "Show in Files" now uses tauri-plugin-opener `revealItemInDir` (mirrors dcpwizard).

## Deliberately skipped

- HDR/WCG ST 2067-21 essence metadata: the carrier now exists (`ImfCpl.essence_descriptors`),
  but this pipeline does not write mastering-display / transfer / colour metadata into
  the picture MXF header (asdcplib AS-02 descriptors here don't carry it, and it can't
  be read back from a J2K codestream, see to_dcp.rs module docs). Emitting an
  RGBADescriptor into the CPL from user-supplied flags alone would describe HDR the
  actual essence doesn't structurally back, which Photon's CPL-vs-MXF conformance would
  reject. So the "source honestly AND validate" bar isn't met; skipped until the MXF
  wrapper writes the metadata it would claim.
- SL (sign language) accessibility: a video-overlay track, not audio; no MCA audio
  descriptor. Composition-level LocaleList still carries languages.
- Non-SCC subtitle conversion: authored TTML/IMSC passes through unchanged, but
  other subtitle inputs become plain timed text and lose regions, placement,
  and styling. Preserve those fields where the source format supplies them.
- slate is a black text slate only; preview plays via mpv (no thumbnails);
  partial-version copies files by CPL uuid (no reel logic); loudness measures only.
  Docs describe these honestly.

## Dedup not done (needs postkit API work or later phase)

- timecode.rs is a `Timecode` struct with methods; postkit::timecode is free functions.
  Not a drop-in; needs a postkit Timecode type first. Left as-is.
- frame_compare.rs: no clean postkit home (later phase, do not touch).

## Keep in sync with dcpwizard (deliberately duplicated, no clean shared home)

Final dedup pass (2026-07-20): shared *logic* already lives in postkit
(mpv::MpvPlayer, packaging writers, escape_xml, parse_srt,
pipeline::run_encode_with_ratio). What remains duplicated is app/framework glue
with no clean cross-repo home. Edit one side, mirror the other:

- gui/src-tauri/src/preview_server.rs — near-identical (only the MpvPlayer app
  name differs). NOT moved to postkit: all `#[tauri::command]` wrappers and postkit
  has no tauri dep (used by CLI + wasm too), so hosting there would force tauri onto
  the core lib. The reusable part is already in postkit::mpv. Note dcpwizard also
  keeps a windows preview_server_stub this repo doesn't.
- gui/src/preview.js, gui/vite.config.js — frontend (differ only by var order / dev
  port); GUIs don't consume JS from the postkit crate, no home.
- gui/src-tauri/src/lib.rs, gui/src-tauri/src/pipeline.rs — app-specific tauri setup
  and build orchestration; encode already delegates to postkit::pipeline. Diverged
  enough (lib.rs module names + terminal guard, pipeline.rs 382 vs 467 lines) that
  unifying would need config flags per divergence.
- .github/workflows/release.yml, gui-release.yml — copies across imfwizard,
  dcpwizard, dcpdoctor differing by binary/artifact names + per-app build deps
  (this repo adds xerces-c/libxml2 + vcpkg steps). Separate repos, no shared
  reusable-workflow without a central repo. Keep aligned by hand.
- Grok CI 2026-07-21: imfwizard does not link grok-ffi, it runs grk_compress at
  encode time. ci.yml `rust` gained the shared cached "Setup grok" step (build
  grok v20.3.6 from source, put grk_compress on PATH) on Linux + macOS so the
  encode path is exercisable; windows is unchanged (no grk_compress, smoke skips
  it). release.yml/gui-release.yml only compile the binary (no encode run), so
  they were left unchanged.
- tests/cli_flags_test.sh — NOT the same harness as dcpwizard's (this one parses
  main.js for sidecar calls; dcpwizard runs the binary and checks clap parse errors).
  Different CLIs, leave separate.
