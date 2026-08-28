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

## Open: `to-dcp` cannot take an IMP imfwizard made

`to_dcp.rs`'s `check_j2k_dci` accepts only a DCI 2K/4K cinema profile, since the
command rewraps essence rather than transcoding it. Every IMP `create` writes now
carries RGB IMF codestreams, so `to-dcp` refuses its own output naming the
transcode it would need. Making it work means decoding each frame, converting
Rec.709 (or the `--hdr` preset's PQ primaries) to DCI X'Y'Z', and re-encoding
under the cinema profile, which is the whole grok encode again rather than a
rewrap. The pieces exist: `postkit::grok_decoder::decode`,
`postkit::colour::DcdmTransform`, and `grok_encoder` with `apply_xyz_transform`
on and the cinema Rsiz. What is missing is the command deciding between a rewrap
and a transcode and reporting which it did. Until then `to-dcp` serves only an
IMP whose picture is already DCI cinema essence.

## Open: no gamut conversion for a source that is not Rec.709

`create --source-colourspace` takes `rec709` alone. P3, Rec.2020, LogC, ACES and
ACEScg parse and are refused by name, because the only transform postkit has for
each of them is `DcdmTransform`, which lands on X'Y'Z'. An App 2E picture carries
the RGB its essence descriptor declares, so what those sources need is an
RGB-to-RGB conversion into Rec.709 (or into the `--hdr` preset's primaries),
which nothing here or in postkit has. `--source-lut` is refused for the same
reason and would be the cheapest route back: a `lut3d` landing on Rec.709 RGB
rather than X'Y'Z' works today if the flag could say which space it targets.
Shared with postkit, whose `colour.rs` is where the matrix would go.

## Open: no black or frozen picture pass over a finished IMP

postkit's `picture_findings::detect_in_essence` runs ffmpeg's blackdetect and
freezedetect over a finished picture file, and dcpwizard's `report
--scan-picture` renders what it finds per reel. Nothing here calls it: this
repo's `report` is a re-export of postkit's generic severity/category renderer,
so there is no section for a per-track-file finding to go in. The encode already
reports its own findings.

## Open: a subsampled App 2E track file has no decode path

`postkit::grok_decoder` refuses a component that is not 4:4:4 by name, so a
frame-extract or a preview of an App 2E track file carrying 4:2:2 chroma has
nowhere to go. Every picture `create` writes is 4:4:4, so this is only about
essence from elsewhere. Chroma upsampling belongs in postkit's decoder, which is
where its DESIGN_TODO carries the entry.

## Open: an HDR IMP has no preview

`frame-extract` and the embedded preview show Rec.709 SDR picture only.
postkit's `render_imf_frame` refuses ST 2084, HLG, BT.2020 and P3-D65 by name,
so an IMP made with `--hdr pq-bt2020` encodes and wraps but cannot be looked at
here: the display transform for it is a tone map plus a gamut conversion into
sRGB, and neither exists in postkit's `colour` yet. The fix is postkit's, and
its DESIGN_TODO carries the entry with the fixture it wants first.

## Open: a dcpdoctor pass does not yet prove App 2E picture

dcpdoctor's IMF validator reads the AS-02 descriptor and never checks that the
codestream Rsiz is an IMF profile, that ColorPrimaries and
TransferCharacteristic are present, or that the coding UL and pixel layout match
the codestream. It passed the cinema-profile X'Y'Z' IMPs this repo shipped
before `create` was fixed. Until dcpdoctor's App 2E check lands (its
DESIGN_TODO has the five checks), the proof that `create` writes App 2E picture
is `app2e_picture.rs` reading the track file back, not a dcpdoctor pass.

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
