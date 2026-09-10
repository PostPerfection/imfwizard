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

## Open: `to-dcp` takes Rec.709 picture only

`to_dcp.rs` transcodes an IMF profile picture whose track file signals Rec.709
primaries and transfer, or signals neither. P3-D65 and BT.2020 primaries need a
gamut conversion and ST 2084 needs a tone map, and `to-dcp` has neither, so an
IMP made with `--hdr pq-bt2020` or `--hdr pq-p3d65` is refused by name. A DCI 3D
LUT applied during the decode is the shape dcpwizard already uses for the tone
map, and the gamut conversion is the same matrix work as
`--source-colourspace p3d65`.

## Open: three sources still reach no Rec.709 RGB

`create --source-colourspace` converts P3-D65, Rec.2020 and LogC with postkit's
`Rec709Transform`. ACES and ACEScg stay refused: reaching a display space from
scene-referred picture needs a rendering transform, and nothing here runs one
(see "`aces` runs no rendering transform"). `p3` stays refused because its
white is DCI rather than D65 and neither this repo nor postkit adapts a white
point. An `--hdr` picture takes no converting source at all, since every
conversion lands on Rec.709 SDR while the preset's descriptor declares PQ:
converting into the preset's own primaries and transfer is the matrix and curve
postkit does not have.

## Open: MaxCLL and MaxFALL are not schema validated

Every composition claims the 2020 App 2E edition, so `write_cpl` emits MaxCLL
and MaxFALL in `http://www.smpte-ra.org/ns/2067-21/2020`. The only App 2E schema
in the tree is `app2e-2016.xsd`, which Photon vendors, and ExtensionProperties
is `xs:any processContents="lax"`, so xmllint skips both elements instead of
checking them. `cpl.rs` asserts their name, namespace and value directly to
cover the gap. What closes it: a 2020 edition App 2E XSD among the fixtures, and
the `st2067_3_complaint` driver importing it under that namespace.

## Open: `aces` runs no rendering transform (2026-09-10)

`aces` converts AP0 to Rec.709 with an ffmpeg colour matrix and nothing else,
so a scene-referred ACES frame comes out without the RRT and ODT a display
render needs. The ctlrender path that would have run the IDT, RRT and ODT was
removed on 2026-09-10: ctlrender had never run here, AMPAS CTL is packaged for
none of the three runner OSes, and its only test proved the fallback. Closing
it means a rendering transform this repo can prove: build CTL with OpenEXR on
each runner or vendor a binary, or port the ACES 1.3 RRT and a Rec.709 ODT into
postkit's colour code, then a test that reads a rendered frame back against
known output values.

## Open: Dolby Vision FEL and profile 4 have no input (2026-09-09)

The README stopped selling FEL and profile 4 on 2026-09-10. MEL is
done: `dv-convert --target-profile mel` runs `ConversionMode::ToMel`, which the
crate accepts on profile 7 or 8, and an 8.1 RPU comes back profile 7 MEL
(`dv_convert_retargets_to_mel`). FEL is not: a full enhancement layer carries
NLQ residual data that neither `dovi_tool generate` nor the crate's
`GenerateProfile` can synthesise, and nothing in the tree has one. Profile 4 is
worse than untested: `convert_with_mode` takes `To81` on profiles 7, 8 and 5
only, so a profile 4 RPU is refused by the library and the README's profile 4
to 8.1 conversion cannot happen at all. Closing FEL means a real profile 7 FEL
source. Closing profile 4 means the conversion existing first.

## Open: the macOS .dmg is unsigned and unnotarised (2026-09-09)

`release.yml` builds the GUI on `macos-15` and bundles a `dmg`, and nothing
signs or notarises it, so Gatekeeper refuses it on any machine that did not
build it. Closing it needs the user's Apple developer certificate, its
password, the team id and an app-specific password in the repo secrets, and
the `APPLE_*` variables the Tauri action reads set from them.

## Open: the sequenced CPL writer belongs in postkit (2026-09-09)

`cpl::write_sequenced_cpl` builds its own sequence and resource XML and puts it
into the SequenceList `postkit::packaging::ImfCpl::to_xml` leaves empty, because
that writer gives every resource a virtual track of its own and a conformed
composition needs one track playing several resources. The indentation of that
empty SequenceList is the seam, and `a_resourceless_cpl_leaves_an_empty_sequence_list`
is what notices if postkit moves it. Closing it means `ImfCpl` taking a list of
sequences, each with its own ordered resources carrying EntryPoint and
SourceDuration, and `write_sequenced_cpl` shrinking to a caller of that. It
needs a postkit change, so it waits on a postkit release the submodule can be
pinned to.

## Deliberately skipped / standing limitations

- SDI monitoring output. The old `sdi-preview` command set an mpv property
  (`vo-decklink-device`) that mpv does not have, so it played on screen while
  claiming DeckLink output, and it was removed. A real path is ffmpeg built with
  `--enable-decklink` writing `-f decklink`, or GStreamer's `decklinkvideosink`,
  both against the Blackmagic desktop video SDK and driver, so it needs a machine
  with the card to build and prove.
- SL (sign language) accessibility: a video-overlay track, not audio, so it gets no
  MCA audio descriptor. Composition-level LocaleList still carries languages.
- Honest-scope tools: slate is a black text slate only, preview plays JPEG 2000
  through grok and every other source through mpv (no thumbnails), partial-version
  copies files by CPL uuid (no reel logic), loudness measures and adjusts to a
  target (no other processing). Docs describe these as-is.

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
  message: the HDR detail flags requiring `--hdr`, the spelling parsers (hdr
  preset, audio role, delivery profile, source colour space, duration specs,
  rotation, flip, raster),
  the GUI's per-composition `--audio-map` without an audio file,
  `--still-length` without a still and a still without one, an appearance flag
  without `--burn-subtitle`, `--burn-subtitle` without `--video`, and the GUI's
  output-folder-already-holds-an-IMP and already-building-into guards. Moving
  them would need the plan to carry which front end built it.
- From the Storm DCP Studio survey (write-up and the DCP-only items in
  dcpwizard's DESIGN_TODO): the playback overlays, decode resolution control,
  player HUD, crop overlay and subtitle/CC toggles in guikit's preview header
  are verified with the mpv CLI and not yet clicked through in the running
  window, which is part of the GUI pass above. That pass owes both backends: an
  IMP loads the grok player, an MP4 source loads mpv, and each draws the overlays
  its own way.

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
