# Planned

Paths: CORE = rust/crates/imfwizard-core/src, CLI = rust/crates/imfwizard-cli/src/main.rs.

## Fixed 2026-07-20 (silent data loss + doc lies)

- GUI builds no longer drop audio/subtitles/bandwidth: `gui/src-tauri/src/pipeline.rs`
  wires the selected audio and subtitle into `ImpOptions` and converts the
  bandwidth (Mbps) to a J2K compression ratio (via `postkit::pipeline::run_encode_with_ratio`).
- REST API executes jobs: `executor.rs` runs a worker over the queue through the
  same core paths as the CLI (encode/transcode/validate/loudness/create); other
  types fail loud. Wired into `serve`, which also gained `--api-key`. Queue is
  in-memory (documented).
- `daemon` and `batch` CLI commands removed (unrunnable in-memory cross-process queue).
- `supplement` fails loud instead of building a mislabelled standalone IMP.
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
  imfwizard now points at the shared `../../postkit`.

## Still overstated or unimplemented (implement or keep de-advertised)

- Multi-CPL IMP: create writes one CPL; GUI composition tabs are cosmetic and Build
  submits only the active composition.
- Accessibility tracks (AD/HI/SL), multi-language audio / RFC 5646, HDR/WCG ST 2067-21
  metadata: no create-path options.
- SCC (CEA-608) to TTML: subtitle-convert is SRT-only; captions.rs extract_captions unused.
- Delivery presets: listed, never applied by create.
- av-sync compares first PTS only (no --fix); slate is a black text slate only; preview
  plays via mpv (no thumbnails); partial-version copies files by CPL uuid (no reel logic);
  loudness measures only. Docs now describe these honestly.
- Dead modules still present: webhook, plugin, otioz_import, imp_diff, subtitle_retime shim.

## Dedup not done (needs postkit API work or later phase)

- timecode.rs is a `Timecode` struct with methods; postkit::timecode is free functions.
  Not a drop-in; needs a postkit Timecode type first. Left as-is.
- to_dcp.rs DCP CPL/PKL/ASSETMAP writers compute per-asset detail (real ScreenAspectRatio)
  that postkit's DcpCpl hardcodes; only the escaper was switched.
- frame_compare.rs and imp_diff.rs: no clean postkit home (later phase, do not touch).

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
- tests/cli_flags_test.sh — NOT the same harness as dcpwizard's (this one parses
  main.js for sidecar calls; dcpwizard runs the binary and checks clap parse errors).
  Different CLIs, leave separate.
