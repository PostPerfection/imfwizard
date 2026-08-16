# Planned

Paths: CORE = rust/crates/imfwizard-core/src, CLI = rust/crates/imfwizard-cli/src/main.rs.

Genuinely open items are standing limitations and deliberate scope decisions;
everything advertised is wired (done notes below).

## Cross-platform embedded preview (hosts landed 2026-08-13, hardware pass open)

The preview host is a crate in guikit at extern/guikit/rust, both wizards take
it as a path dep, and embedded is the only path. All three hosts are
implemented: linux (GtkGLArea, verified live), macos (NSOpenGLView layered over
the WKWebView) and windows (WS_CHILD window over the WebView2 child, wgl). The
pin here carries them and CI compiles all three platforms green. Neither the
macos nor the windows host has run on real hardware yet, so a hand pass there
is the last step before trusting the preview panel on those builds. Details
live in dcpwizard's DESIGN_TODO under "Cross-platform embedded preview". There
is nothing imf-specific beyond bumping the pin.

## Deliberately skipped / standing limitations

- SDI monitoring output. The old `sdi-preview` command set an mpv property
  (`vo-decklink-device`) that mpv does not have, so it played on screen while
  claiming DeckLink output, and it was removed. A real path is ffmpeg built with
  `--enable-decklink` writing `-f decklink`, or GStreamer's `decklinkvideosink`,
  both against the Blackmagic desktop video SDK and driver, so it needs a machine
  with the card to build and prove.
- postkit compiled twice, fixed 2026-08-12. `postkit` was a path dep on
  `extern/postkit` while `dcpdoctor-core` came from git carrying its own path dep
  on the postkit inside that checkout, so cargo resolved two copies. dcpdoctor
  declares postkit by git now, and `rust/Cargo.toml` and `gui/src-tauri/Cargo.toml`
  each carry a `[patch]` redirecting that git source at `extern/postkit`, so both
  references collapse onto the submodule this workspace already builds. The
  edit-the-submodule-and-rebuild loop is unchanged and `cargo tree -d` reports no
  postkit duplicate. dcpwizard has the same shape.

- SL (sign language) accessibility: a video-overlay track, not audio, so it gets no
  MCA audio descriptor. Composition-level LocaleList still carries languages.
- Photon runs and the generated IMP is clean under it, but only on Linux CI:
  `scripts/fetch_photon.sh` pulls Photon and its dependencies from Maven Central
  (checksums pinned) and CI sets `PHOTON_JAR` there, so `hdr_imp_is_clean_under_photon`
  executes rather than skipping. Still gated: a bare run without the script skips
  it, and macOS/Windows CI never runs it.
- Honest-scope tools: slate is a black text slate only; preview plays via mpv (no
  thumbnails); partial-version copies files by CPL uuid (no reel logic); loudness
  measures and adjusts to a target (no other processing). Docs describe these as-is.
- Preview embeds in the app window on linux through the libmpv render API (state
  in postkit's DESIGN_TODO, shared with dcpwizard). macos and windows have no
  host yet, so the preview is unavailable there rather than opening a separate
  mpv window.

## Open: five source colour spaces have no transform (2026-08-16)

`create --source-colourspace` takes all seven of postkit's `ColourSpace` values,
but only `rec709` and `xyz` are wired. `p3`, `rec2020`, `aces`, `acescg` and
`logc` are refused, because nothing in postkit converts them to X'Y'Z':
`colour::convert_colour` is the ffmpeg `colorspace` filter, which cannot express
X'Y'Z' as a target and refuses ACES/ACEScg/LogC as a source without a 3D LUT, and
`colour::rgb_to_xyz_inplace` hardcodes the Rec.709 matrix. Running a P3 or Rec.2020
source through the Rec.709 path would be silently wrong colour, and pre-converting
to Rec.709 first would clip the wider gamut, so both were rejected in favour of a
loud refusal.

Closing it needs a postkit change, one of:
- a `SourceColour` variant carrying the ffmpeg filter chain, so `p3` and `rec2020`
  can go through `colorspace=` during the stream decode (does not reach the three
  log/ACES spaces, which the filter cannot model either);
- real per-space matrices beside `rgb_to_xyz_inplace`, matching what it already
  does for Rec.709;
- or a `--source-lut` flag in both wizards feeding the existing
  `SourceColour::DciLut`, which is postkit's designed route for exactly this.

dcpwizard needs the same decision, so this is a coordinated change across both
repos plus postkit, not an imfwizard-only fix.

## Open: --audio-map reaches only the WAV that is named (2026-08-16)

`--audio-map` maps only the WAV `--audio` names. The track demuxed from `--video`
is written after the map would have run, so it is refused by name rather than
mapped; moving the demux ahead of the map would fix it and is a change to the
order `create` builds its sources in.

## Open: smaller gaps in the source edits (2026-08-16)

- PK wav_io round-trips samples through normalised f32, so 32-bit int PCM is not
  bit-exact through audio delay or trim; 16-bit, 24-bit and float are. The fix is
  postkit's WAV read/write path, queued to land with the next postkit pass so both
  wizards bump pins once.

- Trim happens after the encode, since `postkit::pipeline` takes no frame range.
  Trimming five minutes out of a two-hour source still encodes the two hours.
  A `first_frame`/`frame_count` pair on `EncodeRunOptions` would fix it, again in
  postkit.
- A jpeg image sequence into `encode`/`create` is refused: those frames go to
  grok's own loader through `postkit::encode::detect_image_format`, which has no
  jpeg arm, even though the README's encoder bullet claims JPEG. A lone jpeg still
  is fine, since a held still is decoded by ffmpeg. easyDCP advertises JPEG and BMP
  input (surveyed 2026-08-16, dcpwizard DESIGN_TODO batch E has the full survey).
- GPU J2K encoding. easyDCP and DCP-o-matic both offer GPU/CUDA acceleration;
  nothing here touches it. postkit grok_encoder names no device, and the concept
  exists only on the decode side (preview's gpu_device). Check what the pinned grok
  exposes through grok-ffi before scoping. Shared with dcpwizard, one postkit fix.
- Burn sources are narrower than dcpwizard's: SRT, ASS/SSA, SCC, FCPXML and
  MKS/MKV, the formats there is a cue reader for. TTML/IMSC, the one `--subtitle`
  packages, has no reader anywhere and is refused by name. PAC and Interop
  DCSubtitle have postkit parsers, but imfwizard packages neither format, so
  claiming them only for the burn would leave `subtitle-convert` behind.
- The GUI applies one trim, delay, colour space, picture plan and audio map to
  every composition in a build, because they live in the shared Properties panel. Per-composition values
  would need them on the CPL tab instead. The batch delivery panel deliberately
  sends none of them: it picks its own source in `del-video`, so the Build tab's
  trim and still length describe a different file.
- A trim refuses timed text that carries timing on anything but a `<p>`, because
  TTML times a timed element's children relative to it and resolving that tree is
  more than a trim needs. Authored IMSC that times a `<div>` or animates with
  `<set>` has to be flattened first.
- The GUI's Source Color Space select lists only rec709 and xyz, the two that
  work. It grows when the transforms above land.
- Not every pre-encode check moved into `preflight::check_before_encode`. The
  ones still in the front ends are the flag and control shape checks, each of
  which names the flag or control that carried it and so cannot share one
  message: the HDR detail flags requiring `--hdr`, the `--hdr` plus transforming
  source colour space guard, the spelling parsers (hdr preset, audio role,
  delivery profile, source colour space, duration specs, rotation, flip, raster),
  `--audio-map` without `--audio` (and the GUI's per-composition wording of it),
  `--still-length` without a still and a still without one, an appearance flag
  without `--burn-subtitle`, `--burn-subtitle` without `--video`, and the GUI's
  output-folder-already-holds-an-IMP and already-building-into guards. Moving
  them would need the plan to carry which front end built it.
- From the Storm DCP Studio survey (2026-08-16, full write-up and the DCP-only
  items in dcpwizard's DESIGN_TODO): a codestream forensics section in the QC
  report (decomp levels, precincts, tile-parts, POC, MCT, worst frame vs cap).
  `report` asks dcpdoctor for it with `scan_every_frame`, which the DCP verify
  path reads and the IMF one does not, so an IMP's report carries the measured
  picture bitrate `check_picture_details` gives and no forensics line until
  dcpdoctor's IMF path runs the codestream scan too. The playback overlays (safe
  area, aspect mask, centre cross, thirds grid), the decode resolution control
  (full/half/quarter through the J2K decoder's `lowres`), the player HUD
  (frame, fps, buffer depth, dropped frames), the crop overlay and the
  subtitle/CC render toggles landed in guikit's preview header,
  shared with dcpwizard, verified with the mpv CLI and not yet clicked through
  in the running window. The CLI did
  have dcpwizard's missing-bitrate-flag hole and `create --bitrate <Mbps>`
  closes it. The post-build Play/Inspect/Reveal buttons and the per-stage
  `[TIMING]` lines in the job log are done. What the timings do not give is the
  breakdown inside an encode, colour convert against frame prep against the J2K
  encode itself: postkit's `PipelineProgress` carries a stage name, a frame
  count and an elapsed clock, and nothing that separates the three, so it would
  take a wider progress payload from postkit first.
  Preview subtitles stop at the source picture: the Sub button carries the
  composition's timed text written out as SRT, and after a build nothing unwraps
  the packaged AS-02 timed text track back to cues, so a built IMP plays with
  no subtitles.

## Open: DCP-o-matic hints not ported (2026-08-16)

The hint set in `hints.rs` takes the DoM checks that mean something for an IMP.
The rest are deliberately out:

- Projector frame-rate hints (25, 30, 48/50/60 fps "not supported by all
  projectors"). IMF is not a projection format and allows far more rates than
  DCI does, so porting them would warn about every legal delivery.
- Container ratio hints (Scope content in a Flat container and back, unusual
  ratios). An IMF composition has no container ratio: `--raster` picks one of the
  four App 2E rasters and the picture is fitted into it, which is already refused
  rather than hinted when it does not fit.
- Interop hints (SMPTE-versus-Interop advice, Interop font size, overlapping
  Interop closed captions). imfwizard writes SMPTE only.
- Certificate hints (signing chain UTF-8 strings, validity over 15 years). They
  are about DoM's stored configuration, not about a job, and imfwizard takes a
  cert and key per invocation rather than holding one.
- Reel-splitting hints (text asset larger than 115 MB, caption XML over 256 KB,
  more than 4096 PNG subtitles per reel, missing FFEC/FFMC markers). imfwizard
  writes one reel per composition and has no marker or split concept yet.
- Audio channel-count hints (fewer than 6 channels, not 8 or 16). The IMF sound
  layout comes from the MCA labels and `--audio-map`, and a QC rule about how
  many lanes a distributor expects belongs in the compliance profiles, not here.
- The 4K 3D, upmixer, VOB, alpha and mixed-encryption hints have no imfwizard
  feature behind them at all.

Two more are worth doing when the pieces exist. A large source-to-composition
frame rate difference (DoM warns about audio pitch) needs the retime work that
does not exist here. Closed captions are not packaged as a separate track, so
the 32-character caption rules have nothing to run on.

# Done

## Fixed 2026-08-16 (--hdr guard read the flag, not the route)

`--hdr` without an explicit `--source-colourspace` skipped the already-X'Y'Z'
guard, so `--hdr pq-bt2020` alone ran the encoder's X'Y'Z' transform over
essence it declared untransformed. The guard now reads the resolved
`SourceColour` (rec709 when the flag is absent) and refuses with the same hint,
and the burn refusal's `frames_already_xyz` is that one condition too.

## Fixed 2026-08-12 (postkit c6406d1: one asset id, and a clean Photon run)

- postkit submodule bumped 05516cd -> c6406d1. `MxfWrapOptions` and
  `StereoscopicWrapOptions` gained `asset_uuid`, breaking every struct literal.
  imfwizard constructs no `StereoscopicWrapOptions`, so the breaks were five
  `MxfWrapOptions` sites: mxf_wrap.rs (the delegate and its test), to_dcp.rs x2,
  atmos.rs.
- The MXF carried an id the package never mentioned. `imp.rs` and `supplement.rs`
  minted a uuid for the output filename, then wrote postkit's separately minted id
  into the CPL, PKL and ASSETMAP, so `VIDEO_<a>.mxf` held AssetUUID `<b>` and `<a>`
  appeared nowhere else. Both now pass the minted id down as `asset_uuid`.
  to_dcp.rs and atmos.rs pass None on purpose: they use fixed output names
  (picture.mxf, sound.mxf, atmos.mxf) and either use postkit's returned id or
  discard the track file, so there is no second id to reconcile.
- `track_file_id_is_the_same_everywhere` checks the file name, CPL TrackFileId, PKL
  asset Id, ASSETMAP asset Id and the AssetUUID in the MXF are one value. The MXF
  side is read back from the file bytes: asdcp-info refuses AS-02 ("Inspection in
  not supported by this command") and every IMF track file is AS-02, so it looks
  for the raw 16 bytes the way postkit's own tests do.
- postkit now emits `HashAlgorithm` for the IMF PKL namespace, so Photon gets past
  the PKL for the first time and reports no errors or warnings on any of the four
  files, CPL and picture MXF included. `hdr_imp_is_clean_under_photon` (renamed from
  `hdr_imp_analyzes_under_photon`) now asserts that instead of only asserting Photon
  ran. asdcplib stays at 6d7b8ca, which is what postkit c6406d1 pins.

## Fixed 2026-08-12 (PKL Hash encoding)

- PKL `Hash` for MXF assets was hex where the field is `xs:base64Binary`. postkit's
  `mxf_wrap` returns a hex SHA-1 and `pkl.rs` passed it straight through, while the
  CPL hash on the same path was base64. Photon never caught it: 40 hex characters
  are valid base64, they just decode to 30 meaningless bytes.
- `mxf_wrap.rs` now decodes the hex to the raw digest and base64s that at the postkit
  boundary, so `MxfTrackFile.hash` is base64 for every caller (imp and supplement both
  wrap through it). Decoding rather than sniffing means a postkit that switches to
  base64 fails the wrap loudly instead of double-encoding into a plausible-looking
  wrong value.
- `pkl_hashes_are_base64_sha1_of_the_files` asserts both the CPL and MXF entries
  decode to 20 bytes and equal a SHA-1 computed in the test, so the two paths cannot
  drift apart again.
- Audited every other place imfwizard puts a hash in XML: `to_dcp.rs` uses
  `dcpdoctor_core::hash::sha1_base64` for the DCP PKL CPL and asset hashes (correct),
  ASSETMAP and the IMF CPL carry no hash element, and `signature.rs` delegates digests
  to postkit's xmldsig. Nothing else needed changing.

## Fixed 2026-08-12 (Photon actually runs)

- `photon.rs` invoked `java -cp <single jar>`, which cannot work: Photon ships no
  fat jar and needs slf4j, regxmllib and jaxb-runtime alongside it, so every run
  died with `NoClassDefFoundError: org/slf4j/LoggerFactory`. The launch-failure
  guard caught it, meaning `--photon` had never once produced a verdict. `--photon-jar`
  and `PHOTON_JAR` now also accept a directory, passed to java as `dir/*`, which is
  the layout Netflix documents.
- `scripts/fetch_photon.sh` fetches Photon 5.0.1 and 12 runtime jars from Maven
  Central into a cache directory, sha256 pinned, then smoke-tests that IMPAnalyzer
  starts. Checksums are pinned in the script because Photon 5.0.1's own published
  .sha1/.sha256/.md5 on Maven Central do not match the artifact Central serves;
  every other pin was cross-checked against its publisher sidecar. aws-java-nio-spi-for-s3
  is left out: it is only reached for s3:// inputs and drags in the whole AWS SDK.
- ci.yml `rust` gained cached "Setup Photon" + Temurin 21 on Linux, so
  `hdr_imp_analyzes_under_photon` runs there instead of skipping. No credentials
  needed. Not wired on macOS/Windows.
- README pointed at `photon-all.jar` from Netflix's GitHub releases. That file does
  not exist and those releases carry no binaries at all.
- Found while testing: plain `validate` (no `--photon`) runs a second Photon pass
  inside dcpdoctor-core, which clones Netflix/photon and gradle-builds it unless
  `PHOTON_DIR` points at a jar directory. On a JDK newer than 21 that build fails
  and a whole gradle stack trace lands inside one `warning:` line. Setting
  `PHOTON_DIR` avoids it, which CI and the README now do. The truncation and the
  JDK ceiling are dcpdoctor's to fix.

## DoM tracker gaps (2026-07-22)

The DCP-o-matic Mantis sweep (dom#N = https://dcpomatic.com/bugs/view.php?id=N)
is mostly DCP-side; the items that map here:

- Loudness adjustment to a target (dom#1382): DONE 2026-07-23.
  `loudness --adjust-to <lufs> -o <out.wav>` wires postkit `adjust_loudness`;
  clip-safe true-peak guard via `--true-peak` (default -1 dBTP), refuses and
  writes nothing on a ceiling breach, reports headroom. `#[command(allow_negative_numbers)]`
  so `-24` parses. CLI integration tests with a synthetic 1 kHz WAV cover the
  adjust and clip-refuse paths.
- More subtitle input formats via postkit parsers: FCPXML (dom#2909), ASS with
  styling (dom#1462), MKS (dom#3131). DONE 2026-07-23.
  `subtitle_convert.rs` reads ass/ssa/fcpxml/mks via
  `postkit::subtitle_formats::{ass,fcpxml,mks}`. TTML/IMSC target emits IMSC
  regions (per distinct alignment/position) + inline `tts:` styling from
  `StyledCue`; plain targets flatten via `to_srt_cues`. SCC/authored-TTML paths
  unchanged. Per-format tests with small fixtures (MKS skips without ffmpeg).

## Fixed 2026-07-23 (extern/postkit sync)

Synced `extern/postkit` to the canonical tree (be89fe0: loudness gain API,
subtitle_formats, timecode superset, frame_compare, packaging annotation fields).
Call sites updated for the aa9f01b -> be89fe0 API changes:

- `mxf_wrap::MxfWrapOptions` gained `resource_ids: Vec<[u8;16]>` (caller-supplied
  timed-text ancillary ids): added `resource_ids: vec![]` in mxf_wrap.rs (1) and
  to_dcp.rs (2).
- `packaging::{AssetMap, PackingList}` gained `annotation: Option<String>`: added
  `annotation: None` in assetmap.rs, pkl.rs, and to_dcp.rs (2).
- `certificate::KdmConfig` gained `annotation: Option<String>`: added
  `annotation: None` at the CLI kdm call site.

mid-side wav decode and resumable encode pipeline changes needed no call-site edits
(imfwizard re-exports `encode::{...}` unchanged and does not touch mid-side/stream).
The postkit submodule is pinned at c6406d1.

## Fixed 2026-07-23 (HDR/WCG ST 2067-21 essence metadata)

`create --hdr <preset>` now writes HDR/WCG picture metadata, lifting the long-standing
skip. Presets: `pq-bt2020`, `pq-p3d65` (ST 2084 transfer + BT.2020 / P3-D65 primaries).
Optional `--mastering-display <x265 string>` adds the ST 2086 block; it is refused
without `--hdr`.

- `hdr_wcg.rs`: `HdrWcg` parses the preset + x265 master-display string, converts to
  `asdcplib::jp2k::HdrMetadata`, and emits the CPL RGBADescriptor body (transfer/colour
  ULs as `urn:smpte:ul:`, ST 2086 mastering display).
- MXF write: threaded through `mxf_wrap`/`imp` into postkit's `wrap_j2k`, which calls
  `as02::jp2k::MxfWriter::open_write_hdr` (postkit `MxfWrapOptions` gained an `hdr` field).
- CPL: `cpl.rs` emits the image EssenceDescriptor + SourceEncoding only when `--hdr` is
  set, so the CPL claims only what the MXF carries.
- Tests: round-trips the picture MXF via `hdr_metadata()` (ULs + mastering asserted) and
  xmllint-gates the HDR CPL against imf-cpl-20160411.xsd. A `PHOTON_JAR`-gated test runs
  Photon over the HDR IMP when a jar is present.
- asdcplib pin bumped to 6d7b8ca (the HDR commit). dcpdoctor bumped its own asdcplib
  pin to 6d7b8ca too (at dcpdoctor 171136e), so the temporary workspace `[patch]`
  workaround was dropped; dcpdoctor-core is consumed at rev 6037768 (which includes
  that bump).

## Fixed 2026-07-22 (image-sequence export)

- Export a composition to an image file sequence (dom#3021): new `export-frames`
  CLI command (`-i` IMP dir, `-o` output dir, `--format tiff|png`, `--cpl
  <uuid-or-index>`, `--start`/`--count`). CORE `export_frames.rs` selects a CPL
  (reusing `timeline::list_cpls`/`get_timeline`), walks the composition timeline
  across segments, reads each picture track's AS-02 J2K MXF via asdcplib, and
  decodes every frame's codestream with grk_decompress to numbered files
  (frame_000001.tif). No colour transform: output keeps the codestream's colour
  encoding, so XYZ essence exports as raw code values (dark/green in an RGB
  viewer, intended). TIFF preserves native 10/12-bit; PNG is 8/16-bit only, so
  higher depths pad to 16-bit. Fails loud on encrypted essence (no KDM support).
  DPX is not offered: grk_decompress cannot emit it and an ffmpeg fallback would
  promote to 16-bit, breaking the native-bit-depth contract; `--format dpx` errors
  with that reason. Tested end to end (build IMP from synthetic frames, export,
  assert count + per-file decodable dimensions, subrange, and encrypted rejection).

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

## Dedup onto postkit (done 2026-07-23)

- timecode.rs: DONE. Local copy deleted; lib.rs now
  `pub use postkit::timecode`, so `imfwizard_core::timecode::Timecode` resolves to
  postkit's superset type. Callers (CLI) unchanged.
- frame_compare.rs: DONE. Local copy deleted; lib.rs
  now `pub use postkit::frame_compare`. postkit's module was byte-identical
  (same FrameMetric/CompareResult, compare_frames/compute_vmaf/ffmpeg_has_libvmaf).
  Callers (lib.rs, CLI) unchanged.

## Keep in sync with dcpwizard (deliberately duplicated, no clean shared home)

Final dedup pass (2026-07-20): shared *logic* already lives in postkit
(mpv::MpvPlayer, packaging writers, escape_xml, parse_srt,
pipeline::run_encode_with_ratio). What remains duplicated is app/framework glue
with no clean cross-repo home. Edit one side, mirror the other:

- gui/src-tauri/src/preview_server.rs and preview_surface.rs — moved to the guikit
  crate 2026-08-13, no longer duplicated. Both wizards depend on
  extern/guikit/rust and register its commands. It did not go to postkit, which
  must stay free of a tauri dep since the CLI and wasm use it too.
- preview.js — no longer duplicated, both wizards import extern/guikit/src/preview.js
  (2026-08-13). Edit it in guikit and bump the pin in both.
- gui/vite.config.js — still per-app (dev port differs) but now partially aligned:
  the `server.fs.allow` and `resolve.dedupe` blocks are what make the out-of-root
  guikit sources resolve, and both wizards need them identical.
- shortcuts.js — deleted from both wizards, they import
  extern/guikit/src/shortcuts.js (2026-08-13). dcpdoctor stays out of guikit and
  keeps its vendored copy, synced by cp. App-agnostic by design: all app specifics
  enter through initShortcuts, so an integration never needs an engine change; if
  one seems to, change the engine in guikit and cp to dcpdoctor.
- gui/src/timeline.js — classified 2026-08-13 as genuine domain difference, not
  drift, so it stays per-app deliberately. It is a thin renderer over disjoint
  backend structs (SegmentEntry segments here, TimelineEntry reels in dcpwizard),
  and the zoom control and IMP argument builder are imfwizard-only.
- gui/src-tauri/src/lib.rs, gui/src-tauri/src/pipeline.rs — app-specific tauri setup
  and build orchestration; encode already delegates to postkit::pipeline. Diverged
  enough (lib.rs module names + terminal guard, pipeline.rs 382 vs 467 lines) that
  unifying would need config flags per divergence.
- .github/workflows/release.yml, gui-release.yml — copies across imfwizard,
  dcpwizard, dcpdoctor differing by binary/artifact names + per-app build deps
  (this repo adds xerces-c/libxml2 + vcpkg steps). Separate repos, no shared
  reusable-workflow without a central repo. Keep aligned by hand.
- Grok 2026-08-16: imfwizard links grok-ffi, mirroring dcpwizard (one workspace
  dependency line; the gui workspace picks it up through imfwizard-core's
  workspace-inherited postkit dep, feature-unified). libgrokj2k is therefore a
  build requirement everywhere, so every job that compiles the workspace carries
  the cached "Setup grok" step: ci.yml `rust` (all three platforms) and `gui`,
  release.yml `build` + `deb`, gui-release.yml. Windows uses the separate msvc
  build of the same tag. The cli zip ships grokj2k.dll beside the exe and
  tauri.windows.conf.json bundles it into the installer; a local windows tauri
  build fails at bundle time unless the dll is staged at gui/src-tauri/grokj2k.dll.
  Unproven until the next tag run: if grok's msvc install drops more dlls that
  grokj2k.dll depends on, copy bin/*.dll in both places instead.
- tests/cli_flags_test.sh — NOT the same harness as dcpwizard's (this one parses
  main.js for sidecar calls; dcpwizard runs the binary and checks clap parse errors).
  Different CLIs, leave separate.
