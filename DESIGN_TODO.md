# Planned

Paths: CORE = rust/crates/imfwizard-core/src, CLI = rust/crates/imfwizard-cli/src/main.rs.

Genuinely open items are standing limitations and deliberate scope decisions;
everything advertised is wired (done notes below).

## Cross-platform embedded preview hardware pass

The preview host is a crate in guikit at extern/guikit/rust, both wizards take
it as a path dep, and embedded is the only path. The macos and windows hosts
have not run on real hardware, so a hand pass there is the last step before
trusting the preview panel on those builds. Details in dcpwizard's DESIGN_TODO
under "Cross-platform embedded preview".

## Open: frame-extract still decodes J2K with ffmpeg

`frame-extract` calls `postkit::preview::extract_frame`, which runs ffmpeg over
whatever it is given, so for an App 2E track file that is ffmpeg's software
jpeg2000 decoder at a few frames a second, and its `-ss` sits after `-i`, so a
late frame decodes every frame before it. postkit now decodes J2K in process
through `grok_decoder`, 68 ms a frame on 2K at 125 Mb/s against ffmpeg's 302 ms
and 5 ms at `reduce` 2, and this workspace already links `grok-ffi`, so the
decoder is here and unused. Routing `extract_frame` to it needs a rule for what
counts as J2K essence and a decision about encrypted essence with no key, which
ffmpeg renders as garbage and grok refuses. Same entry in dcpwizard's
DESIGN_TODO: one postkit change serves both.

## Open: no black or frozen picture pass over a finished IMP

postkit's `picture_findings::detect_in_essence` runs ffmpeg's blackdetect and
freezedetect over a finished picture file, and dcpwizard's `report
--scan-picture` renders what it finds per reel. Nothing here calls it: this
repo's `report` is a re-export of postkit's generic severity/category renderer,
so there is no section for a per-track-file finding to go in. The encode already
reports its own findings.

## Deliberately skipped / standing limitations

- SDI monitoring output. The old `sdi-preview` command set an mpv property
  (`vo-decklink-device`) that mpv does not have, so it played on screen while
  claiming DeckLink output, and it was removed. A real path is ffmpeg built with
  `--enable-decklink` writing `-f decklink`, or GStreamer's `decklinkvideosink`,
  both against the Blackmagic desktop video SDK and driver, so it needs a machine
  with the card to build and prove.
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

## Open: a full GUI pass has never been done (2026-08-17)

Every imfwizard GUI feature so far was verified through tests and dcpwizard's
click-throughs; nobody has walked this app's own GUI end to end by hand. Owed
alongside the shared re-verification: the QC overlays drawn at and across end
of file without freezing and without a frame-rate hit (watch the HUD decoder
fps), the playlist fixes live (clearing rows stops or clears the preview when
the queue owns it, one advance per end of file), and the transport bar tracking
live during playback after postkit's non-blocking render fix (its DESIGN_TODO
has the entry).

## Open: smaller gaps in the source edits (2026-08-16)

- GPU J2K encoding. The grok library has no GPU encode path of its own: it is a
  separately licensed accelerator plugin (`grk_plugin_load` and
  `grk_plugin_init` with a device id and a licence key), which is what
  DCP-o-matic's `config grok-licence` drives. postkit's DESIGN_TODO has the
  scoping. Shared with dcpwizard, one postkit change plus a device and licence
  setting in each wizard.
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
- Not every pre-encode check moved into `preflight::check_before_encode`. The
  ones still in the front ends are the flag and control shape checks, each of
  which names the flag or control that carried it and so cannot share one
  message: the HDR detail flags requiring `--hdr`, the `--hdr` plus transforming
  source colour space guard, the spelling parsers (hdr preset, audio role,
  delivery profile, source colour space, duration specs, rotation, flip, raster),
  the GUI's per-composition `--audio-map` without an audio file,
  `--still-length` without a still and a still without one, an appearance flag
  without `--burn-subtitle`, `--burn-subtitle` without `--video`, and the GUI's
  output-folder-already-holds-an-IMP and already-building-into guards. Moving
  them would need the plan to carry which front end built it.
- From the Storm DCP Studio survey (write-up and the DCP-only items in
  dcpwizard's DESIGN_TODO): the playback overlays, decode resolution control,
  player HUD, crop overlay and subtitle/CC toggles in guikit's preview header
  are verified with the mpv CLI and not yet clicked through in the running
  window, which is part of the GUI pass above.

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
