# IMF Wizard

[![CI](https://github.com/PostPerfection/imfwizard/actions/workflows/ci.yml/badge.svg)](https://github.com/PostPerfection/imfwizard/actions/workflows/ci.yml)

[Documentation](https://postperfection.github.io/imfwizard/)

Interoperable Master Format (IMF) package creator, CLI tool and desktop GUI. Written in Rust.

Version 1.4.0 writes complete CPL, PKL, and ASSETMAP references, uses base64 package hashes, identifies App 2E, and rejects incompatible picture essence before packaging.

## Overview

IMF Wizard creates valid IMF packages (Interoperable Master Packages) from
video sources, image sequences, and WAV audio, conforming to SMPTE ST 2067 (App#2E).

## Features

### Packaging & Wrapping
- **Original Version IMP creation** from J2K + WAV
- **TTML / IMSC subtitle** packaging as AS-02 timed text MXF
- **Subtitle conversion** to IMSC/TTML from SRT, SCC (CEA-608 pop-on captions), ASS/SSA, FCPXML, and MKS (Matroska); ASS/FCPXML/MKS keep styling and placement (italic/bold/underline/colour, alignment, position) in the TTML output
- **App 2E picture essence**, the codestreams declare an IMF JPEG 2000 profile (RSIZ 0x0400 to 0x09ff, the profile the raster picks with the levels its rate and bitrate ask for) and carry 12-bit RGB 4:4:4. The picture MXF signals ColorPrimaries and TransferCharacteristic on its RGBA essence descriptor, Rec.709 without `--hdr` and the preset's PQ or HLG transfer with it
- **AS-02 MXF wrapping** (SMPTE 2067-5), CPL/PKL/AssetMap generation
- **SHA-1 hashing** of every PKL asset, written as base64
- **Optional XML-DSIG signing** of CPL/PKL/ASSETMAP (`sign` / `verify-sig`, needs a cert + key)
- **IMF to DCP**, convert a single-composition IMP (one picture, optional one sound) to a DCP. Picture already in a DCI 2K/4K cinema profile is rewrapped as it stands. Picture in an IMF profile, which is what `create` writes, is decoded, converted from Rec.709 RGB to DCI X'Y'Z' and re-encoded under the cinema profile at `--bitrate` Mb/s, up to the DCI maximum of 250. The command prints which of the two it did. P3-D65, BT.2020 and PQ picture is refused naming the gamut conversion or tone map it would need, and so is an edit rate outside the DCI set (24, 25, 30, 48, 50, 60, 96, 100, 120)

### Encoding & Transcoding
- **Image encoding pipeline**, DPX, TIFF, EXR, PNG, BMP, JPEG → 12-bit JPEG 2000 through the linked Grok library. TIFF frames are read by imfwizard itself, at 8, 12 or 16 bits, and every other format decodes through ffmpeg first
- **CPU encoding** on all available cores. GPU encoding needs Grok's accelerator plugin, a commercial product sold separately, see [GPU builds](#gpu-builds)
- **Video transcoding via ffmpeg** (`transcode`, pick the output codec, e.g. libx264/prores)
- **ProRes encoding** (`prores`), encode a video file or a directory of numbered frames (at the frame rate `--fps-num` and `--fps-den` name, required for one since stills carry no rate) to a ProRes .mov master, or export an IMP as a ProRes 4444 delivery master fitted into a named cinema container
- **Burn-in during the encode**, `create --burn-subtitle <file>` (+ `--burn-subtitle-font <ttf/otf>`) draws the cues into the picture as it encodes, so a burnt master costs one generation rather than two. Reads SRT, ASS/SSA, SCC, FCPXML and MKS/MKV, and covers video, image sequences and held stills. Burnt text is part of the image and registers no timed-text track, the same file cannot be both, and burning onto a J2K directory is refused
- **Burn-in appearance**, `create --burn-font-size`, `--burn-colour`, `--burn-effect none|outline|shadow`, `--burn-effect-colour`, `--burn-outline-width`, `--burn-line-height`, `--burn-margin`, `--burn-x-scale`, `--burn-y-scale`, `--burn-fade-up` and `--burn-fade-down` set how the burnt text looks. A flag left out keeps the default, and any of them without `--burn-subtitle` is refused by name. The Properties panel carries all but the two scales
- **Subtitle burn-in as a standalone pass**, `burn-in` renders SRT or ASS into video frames via ffmpeg, outside a package
- **Trim**, `create --trim-start` / `--trim-end` take frames (`48f`) or seconds (`2s`) off the head and tail; picture, sound and timed text move together, and cues outside the kept range are dropped or clamped
- **Source picture processing**, `create --crop-left/--crop-right/--crop-top/--crop-bottom` cut source pixels off each side, `--auto-crop` (`--auto-crop-threshold`) measures the black borders and cuts them, `--fill-crop` crops to the target aspect instead of padding to it, `--deinterlace`, `--denoise`, `--rotate 90|180|270`, `--flip horizontal|vertical|both`, and `--raster <WxH>` fits the result into one of the App 2E rasters. Anything other than the untouched source is fitted into a raster, so the App 2E check runs on what the encoder writes rather than on the source
- **Still image with duration**, `create --still-length` holds a single image (dpx, tif/tiff, exr, png, jpg/jpeg, bmp) for that long, encoding it once and repeating the codestream. With `--burn-subtitle` the repeat breaks only where the cues change, so the hold costs a handful of encodes rather than one per frame

### HDR & Advanced
- **HDR/WCG essence metadata (ST 2067-21)** — `create --hdr pq-bt2020|pq-p3d65|hlg-bt2020` writes the transfer/colour ULs onto the picture MXF RGBA descriptor and the CPL EssenceDescriptor. The CPL entry is the track file's whole descriptor, read back out of the MXF after the wrap and written as RegXML, so Photon's field for field comparison of the two passes. A picture with no `--hdr` declares Rec.709, App 2E COLOR.3. `hlg-bt2020` is App 2E COLOR.8, the BT.2020 primaries with the HLG OETF. Optional `--mastering-display` adds the ST 2086 block, and `--max-cll` / `--max-fall` add the content light levels as CPL ExtensionProperties
- **Dolby Vision** RPU metadata injection (via dovi_tool)
- **HDR10+ dynamic metadata injection** (`hdr10plus-inject`) writes an HDR10+ JSON into an HEVC elementary stream, and `hdr10plus-extract` reads it back out (both via hdr10plus_tool)
- **HDR10 static metadata** injection through `hdr10-inject`, re-encodes with libx265 to write the mastering display and content light level SEI
- **Dolby Atmos / immersive audio packaging**, `create --atmos <dir>` wraps a directory of IA bitstream frame files, one per picture frame, as an ST 2067-201 IAB track file and plays it from the CPL as an IABSequence. The GUI does not offer it yet. The `atmos` import carries ADM channels as PCM MXF and converts nothing to IAB

### Quality Control
- **Pre-build check**, `create --check` runs every refusal `create` can make and prints the advisory hints, then stops without encoding or writing anything under `--output`. Every refusal that could once fire after the encode (an illegal raster on a J2K directory, a trim or delay longer than the source, timed text a trim cannot move, a bad audio map) now fires here first
- **Pre-build hints**, advisory findings that build but are usually wrong for the audience: audio true peak above -3 dBTP, sound with no language set, a first subtitle before 4 seconds, a cue under 15 frames, cues less than 2 frames apart, more than 3 lines in a cue, and lines over 52 or 79 characters. The CLI prints them before the encode, the GUI shows them in a dialog you can build through
- **Loudness analysis**, EBU R128 integrated/true-peak measurement, and `loudness --adjust-to <LUFS> -o <out.wav>` writes the level-adjusted WAV
- **XSD schema validation**, `validate --xsd` checks the CPL, PKL, AssetMap and OPL with xmllint against the ST 2067-3, ST 2067-2, ST 429-9 and ST 2067-100 schemas the binary carries, copied from the SMPTE registry
- **Structural validation** via dcpdoctor-core (ASSETMAP/PKL/hash checks) plus CPL/PKL signature verification
- **Verified on the way out**, `create` runs that same validation over the package it just wrote and exits non-zero on any error, leaving the package in place to look at. `create --no-verify` skips it and says so. A desktop build runs it as its last stage and writes the findings into the job log, unless Settings turns it off
- **Netflix Photon validation** (optional, needs a JRE and the Photon jars): `validate` runs it whenever `--photon-jar` or `PHOTON_JAR` names them, and `validate --photon` prints its own Photon pass or fail line
- **PSNR / SSIM** frame comparison between two image sequences, two video or MXF files, or the picture track files of two IMPs (`compare --pixel`)
- **VMAF** (optional) via `compare --vmaf` (needs an ffmpeg built with libvmaf)
- **Bitrate analytics**, per-second throughput, histogram, standard deviation (JSON output for dashboards)
- **QC report** generation (text / JSON / HTML), with optional black and frozen picture detection via `report --scan-picture`
- **Black and frozen runs while encoding**, opt in with `create --detect-picture-findings` or the desktop *Report black and frozen runs while encoding*, saved as `detectPictureFindings`. ffmpeg's `blackdetect` and `freezedetect` run on a branch of the decode, and each run of 2 seconds or more is logged as a warning. It is off by default because the branch runs on ffmpeg's single filter thread, which slows a GPU encode. `--no-detect-picture-findings` overrides the saved setting, and the desktop job log prints `Picture findings: on` or `off`
- **Platform compliance checking** (ffprobe-based) against smpte, netflix, disney, hbo, dolby, dci-2k, dci-4k, archival and broadcast

### Color & Audio Processing
- **Source colour space**, `create --source-colourspace rec709` says what the picture carries. An App 2E picture ships the Rec.709 RGB its essence descriptor declares, so rec709 (the default) is compressed untouched, and p3d65, rec2020 and logc are converted to Rec.709 RGB frame by frame during the encode. xyz is refused because a DCI codestream is a DCP picture rather than an IMF one, p3 because P3 with the DCI white needs a white point adaptation nothing here does (name p3d65 for a D65 master), and aces and acescg because reaching Rec.709 from them needs a rendering transform rather than a matrix. A converting value together with `--hdr` is refused too, since the conversion lands on Rec.709 SDR and the `--hdr` descriptor declares PQ
- **Source LUT**, `create --source-lut <file.cube>` applies a 3D LUT during the decode, and its output must be Rec.709 RGB. It conflicts with `--source-colourspace`, and it needs a decode to run in, so a held still is refused
- **Audio delay**, `create --audio-delay <ms>` shifts the sound against the picture without changing the running time, padding one end and truncating the other
- **Audio channel mapping**, `create --audio-map "1:L,2:R,1:C@-6"` routes and mixes the source channels into named lanes (L, R, C, LFE, Ls, Rs, Lrs, Rrs, or 1-based numbers) with a per-route gain in dB. The source is the `--audio` WAV, or the track demuxed from `--video` when there is no `--audio`. Several inputs summed into one lane are mixed. A plain routing is bit-exact. The map runs before the delay, the trim and the MCA labels, so the labelled layout describes the packaged file
- **3D LUT application** (`lut`), apply a .cube LUT to an image sequence or a video file via ffmpeg lut3d
- **ACES conversion**, `aces` converts ACES AP0 frames to Rec.709 with an ffmpeg colour matrix. No rendering transform runs, so this is a colorimetric conversion, not a display render
- **Audio description mixing**, combine AD narration with main mix using ducking
- **MCA label generation**, SMPTE ST 377-4 Multi-Channel Audio labeling (5.1, 7.1, stereo presets)
- **Dolby Atmos ADM BWF import**, parse ADM metadata and wrap the PCM essence to MXF (not a Dolby IAB bitstream)
- **A/V sync detection**, compare per-stream start and end on the container clock for initial offset and drift over the program

### Versioning & Annotation
- **Supplemental IMP** (`supplement`), package only the new/changed track files with a CPL that references the unchanged OV track files by UUID (ST 2067-2/-3 OV+supplemental)
- **CPL annotation**, add revision/text notes to a CPL XML
- **Partial version creation**, copy the files a given CPL UUID references into a new IMP
- **Video retiming** (`retime`), change a video file's frame rate via ffmpeg

### Pre-roll & Leaders
- **Slate generation**, prepend a black text slate as an image sequence

### Integration & Extensibility
- **REST API server** (`serve`), HTTP interface under `/api/v1`: `create`, `validate`, `encode`, `transcode`, `jobs`, `profiles`, `tools`, `pause`, `resume` (in-memory queue with a background worker; jobs live for the server process only)
- **EDL/FCP XML import**, `conform` parses CMX 3600 EDL and Final Cut Pro 7 XML timelines
- **Dependency management (`doctor`)**, check external tool dependencies with version detection and JSON output

### Workflow & Automation
- **Delivery presets**, profiles (Netflix, Disney+, HBO Max, Cinema 2K/4K, ...); apply one to an encode with `create --profile <name>`, or name the target directly with `create --bitrate <Mbps>`
- **Watch folder** (`watch <dir> --output <dir> [--webhook-url <url>] [--interval <seconds>] [-- <create flags>]`), build an IMP from every video file or frame folder that lands in the watched directory, once it stops changing. The file stem is the title, a same-named `.wav` and `.ttml` beside it become the sound and subtitle, the job log is written beside the package, the source moves into `done/` or `failed/` and a webhook gets `imp.created` or `imp.failed`
- **EDL conform**, `conform --input <timeline> --media-dir <dir> --output <imp>` builds an IMP whose CPL follows a CMX3600 or FCP7 timeline, one picture and sound resource per event, trimmed to the event's source range
- **S3 / Aspera / rsync upload** of completed IMPs, with a SQLite delivery tracker
- **Partial restore** (`restore --input <imp> --output <dir> [--video-only|--audio-only]`), unwraps every track file the IMP's CPLs name, in process: a picture track becomes one numbered `.j2c` codestream per frame and a sound track becomes one WAV at the channel count, rate and depth the track file declares, each under a directory named after the track file

### Comparison & Analysis
- **IMF package compare** (`compare`), metadata diff of two IMPs (title, CPL count, duration, edit rate), or with `--pixel`/`--vmaf` pixel PSNR/SSIM/VMAF between the picture track files of the two IMPs' first CPLs, or between two video or MXF files or frame directories, which need `--fps-num` and `--fps-den`
- **Frame extract** (`frame-extract`), write one frame of a video or MXF file as an image. JPEG 2000 MXF (IMF App 2E and DCI cinema) is decoded in process with Grok, and other files go through ffmpeg

### Distributed & Advanced
- **Dolby Vision profile 8.1**, `dv-extract`, `dv-convert --target-profile 8.1|mel` and `dv-inject` retarget and rewrap an RPU. A profile 7 MEL comes from an 8.1 RPU. FEL and profile 4 have no input and are not converted
- **Prometheus metrics**, `/metrics` endpoint on REST API exposing job-state gauges
- **Shell tab completion**, bash, zsh, and fish completion scripts (`imfwizard completion bash`)

### Desktop GUI (Tauri 2)
- **Dark theme** by default with optional light mode toggle
- **File import** (video, WAV, TTML/subtitle) via file picker; a build packages the selected picture, audio and subtitle
- **Keyboard shortcuts**, Ctrl+N/O/S/B/P/I, Ctrl+Shift+S (Save As), Ctrl+Shift+O (Open IMP), Ctrl+Shift+N (Create supplement), Ctrl+1..7 tab navigation and Space/arrows/Home during preview. Ctrl+K opens the shortcut list, where clicking a shortcut rebinds it (Backspace clears, Escape cancels) and the rebindings are saved
- **Progress bars**, real-time progress tracking for encode/wrap jobs
- **IMP metadata editor**, edit CPL title/annotation
- **Preview player** with timeline scrubber (click-to-seek, drag-to-scrub, timecode display). An IMP, a picture track file, a CPL or a directory of codestreams plays through Grok in process. A CPU worker pool handles the decode, the App 2E tone map and the gamut conversion. Everything else plays through mpv
- **GPU encoding toggle**, only useful with the commercial Grok accelerator plugin
- **Subtitle burn-in**, GUI for hardcoding subs into video
- **Picture and audio controls**, per-side crop with an Auto-crop button, fill/deinterlace/denoise, rotate, flip and raster in the Picture section, and a channel mapping matrix in the Audio section
- **Pre-build hints dialog**, Build stops on the advisory findings with Build anyway / Go back, and a "Don't show hints again" checkbox. Settings > General has the same toggle to turn it back on. The findings are also written into the job log
- **Post-build actions**, a finished build offers Play (the new IMP in the embedded preview), Inspect (the Validate view, already pointed at the output and running) and Reveal (the output folder in the file manager), beside the progress bar. Starting another build clears the row
- **Per-stage timings in the job log**, `[TIMING]` lines next to the stage's own log lines giving probe, encode, audio map, source edits and packaging time per composition, plus the total
- **Job queue manager**, submit, monitor, cancel background jobs. The queue is written to `~/.config/imfwizard/gui-jobs.jsonl`, one JSON line per job on submit and on every state change, and read back on start, so closing the window does not lose queued jobs. A job left running when the app closed is listed failed. `$IMFWIZARD_GUI_JOBS_FILE` points a second app at another file
- **Progress notifications**, system notifications when jobs complete
- **Projects**, New (Ctrl+N) asks where the `.imfwizard` file goes and starts from the default panel, titled after the file, with the IMP going in a folder of that name beside it. Save (Ctrl+S), Save As (Ctrl+Shift+S) and Open (Ctrl+O) write and read the whole Properties panel, with the sources, compositions, segments, channel map and output directory but not Settings. A source that moved is looked for beside the file. Every build also writes `<output>.imfwizard` beside the package and saves the open project, Recent lists these files, and unsaved changes are kept as a draft in the app data folder that comes back on the next launch. A file saved by an older version is upgraded on open and rewritten on Save, one saved by a newer version is refused. Open IMP (Ctrl+Shift+O) opens a built package

### Packaging & Deployment
- **Docker image**, headless batch processing (`docker run imfwizard create ...`)
- **macOS .dmg** for Apple Silicon
- REST API mode with Prometheus-compatible `/metrics` endpoint

### Mastering & Compliance
- **DCDM creation**, Digital Cinema Distribution Master (X'Y'Z' 12/16-bit) as intermediate format
- **Visible watermark burn-in**, burn operator/session text into an image sequence
- **Trailer packaging**, ratings cards (MPAA/BBFC/FSK), green/red band, countdown leaders
- **Content version tracker** (`version record|list|export`), SQLite database tracking version history and delivery destinations, with `--db` defaulting to `deliveries.db` in the working directory
- **Accessibility compliance**, verify AD/HI/SL tracks against CVAA, EAA, AODA, Ofcom standards

## Installation

### Pre-built binaries (recommended)

Download from the [GitHub Releases](https://github.com/PostPerfection/imfwizard/releases/latest) page:

| Platform | CLI | Desktop GUI |
|----------|-----|-------------|
| **Linux** (x86_64) | `imfwizard-linux-x86_64.tar.gz` | `.deb`, `.rpm`, `.AppImage` |
| **macOS** (Apple Silicon) | `imfwizard-macos-aarch64.tar.gz` | `.dmg` |
| **Windows** (x86_64) | `imfwizard-windows-x86_64.zip` | `.msi` |

The CLI binary carries everything but the Grok JPEG 2000 codec, which it links dynamically. Every archive ships that library: `grokj2k.dll` beside the exe in the Windows zip, `libgrokj2k` in `lib/` beside the binary in the Linux and macOS tarballs, where the binary's rpath finds it. Extract and run, no loader path to set.

The desktop packages carry libgrokj2k too, in `/usr/lib/imfwizard`. The package manager pulls in the rest: libmpv for the preview player, ffmpeg for video import, xmlsec1 and xmllint for verification, curl for webhook notifications.

```bash
sudo apt install ./IMF.Wizard_*_amd64.deb     # Debian, Ubuntu
sudo dnf install ./IMF.Wizard-*.x86_64.rpm    # Fedora
```

On Fedora, enable [RPM Fusion](https://rpmfusion.org/Configuration) first: ffmpeg comes from there. `dv-extract` and `dv-inject` need `dovi_tool` on the PATH, and `hdr10plus-extract` needs `hdr10plus_tool`, from their GitHub releases listed under runtime dependencies below.

The `.AppImage` carries libmpv as well, and runs ffmpeg, xmlsec1 and xmllint from the PATH. For the `.dmg`, install libmpv with `brew install mpv`.

### Install from source

Every build, the CLI included, links the FFmpeg 8.1.3 LGPL libraries, and the desktop app also links libmpv. Both come from a [PostPerfection/ffmpeg-mpv-builds](https://github.com/PostPerfection/ffmpeg-mpv-builds/releases/tag/v1.0.0) release, v1.0.0 in CI. Unpack the archive for your platform, here to `/path/to/ffmpeg-mpv`. On Linux and macOS put its `lib/pkgconfig` first on `PKG_CONFIG_PATH`, on Windows set `FFMPEG_DIR` to it and `MPV_LIB_DIR` to its `lib`. On Ubuntu 24.04 install the packages its `ubuntu-24.04-runtime-packages.txt` lists. On a machine with a distro FFmpeg also set `FFMPEG_DIR` to it on Linux and macOS: the build scripts put `$FFMPEG_DIR/lib` first on the link path, ahead of `/usr/lib64`.

Every build needs the [Grok](https://grok.rocks/) JPEG 2000 codec, since the picture encoder calls it in-process. Build and install it once, then put it on the pkg-config and loader paths:

```bash
git clone --recurse-submodules --branch v20.4.14 https://github.com/GrokImageCompression/grok.git
cmake -S grok -B grok/build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$HOME/bin/grok"
cmake --build grok/build --parallel
cmake --install grok/build

export PKG_CONFIG_PATH="/path/to/ffmpeg-mpv/lib/pkgconfig:$HOME/bin/grok/lib64/pkgconfig:$HOME/bin/grok/lib/pkgconfig:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="/path/to/ffmpeg-mpv/lib:$HOME/bin/grok/lib64:$HOME/bin/grok/lib:$LD_LIBRARY_PATH"
```

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get install -y build-essential cmake pkg-config libssl-dev libxerces-c-dev libasound2-dev libclang-dev
# For GUI: also install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf

git clone --recurse-submodules https://github.com/PostPerfection/imfwizard.git
cd imfwizard/rust
cargo build --release
# Binary at rust/target/release/imfwizard
```

#### Fedora

```bash
sudo dnf install gcc-c++ cmake pkgconf-pkg-config libxml2-devel openssl-devel xerces-c-devel alsa-lib-devel clang-devel
# For GUI: also install webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel patchelf
# ffmpeg comes from RPM Fusion
export FFMPEG_DIR=/path/to/ffmpeg-mpv

git clone --recurse-submodules https://github.com/PostPerfection/imfwizard.git
cd imfwizard/rust
cargo build --release
# Binary at rust/target/release/imfwizard
```

#### macOS

```bash
brew install pkg-config libxml2 openssl@3 xerces-c

export OPENSSL_DIR=$(brew --prefix openssl@3)
export PKG_CONFIG_PATH="/path/to/ffmpeg-mpv/lib/pkgconfig:$(brew --prefix openssl@3)/lib/pkgconfig:$(brew --prefix libxml2)/lib/pkgconfig:$(brew --prefix xerces-c)/lib/pkgconfig:$PKG_CONFIG_PATH"
export DYLD_LIBRARY_PATH="/path/to/ffmpeg-mpv/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"

cd rust
cargo build --release
```

#### Windows

```powershell
# Using vcpkg (recommended)
vcpkg install libxml2 openssl xerces-c --triplet x64-windows

$env:VCPKG_ROOT = "$env:VCPKG_INSTALLATION_ROOT"
$env:CMAKE_TOOLCHAIN_FILE = "$env:VCPKG_INSTALLATION_ROOT/scripts/buildsystems/vcpkg.cmake"
$env:FFMPEG_DIR = "C:\path\to\ffmpeg-mpv"
$env:MPV_LIB_DIR = "C:\path\to\ffmpeg-mpv\lib"
$env:PATH = "C:\path\to\ffmpeg-mpv\bin;$env:PATH"

cd rust
cargo build --release
```

### Optional runtime dependencies

| Dependency | Purpose | Install |
|-----------|---------|---------|
| `ffmpeg` / `ffprobe` | Video transcoding, loudness, quality metrics | `apt install ffmpeg` / `brew install ffmpeg` / [ffmpeg.org](https://ffmpeg.org/download.html) |
| `dovi_tool` | Dolby Vision RPU injection | [GitHub](https://github.com/quietvoid/dovi_tool/releases) |
| `hdr10plus_tool` | HDR10+ dynamic metadata | [GitHub](https://github.com/quietvoid/hdr10plus_tool/releases) |
| `xmllint` | XSD schema validation of IMP XML | `apt install libxml2-utils` / `brew install libxml2` |
| ffmpeg with `libvmaf` | VMAF in `compare --vmaf` | ffmpeg built `--enable-libvmaf` (check `ffmpeg -filters \| grep libvmaf`) |
| JRE + Photon jars | `validate --photon` (Netflix Photon) | `apt install default-jre`; then `scripts/fetch_photon.sh` |
| `ascp` | Aspera FASP high-speed transfer | [IBM Aspera](https://www.ibm.com/aspera) |
| AWS CLI | S3 upload | [docs.aws.amazon.com](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html) |

The desktop app needs libmpv: the .deb and .rpm pull it in, the AppImage carries it, on macOS run `brew install mpv`. It plays sources that are not JPEG 2000.

Use `imfwizard doctor` to check which tools are installed and which are missing.

### Docker

```bash
docker build -t imfwizard .
docker run -v /path/to/media:/data imfwizard create \
    --title "My Film" --video /data/j2k --audio /data/audio.wav --output /data/imp
```

### Desktop GUI (Tauri 2)

The desktop app uses a single-window layout with sidebar navigation, inspired by professional NLEs.

```bash
./scripts/setup-tauri-bin.sh
# macOS: lib/libgrokj2k.1.dylib, Windows: bin/grokj2k.dll
cp -L "$HOME/bin/grok/lib64/libgrokj2k.so.1" gui/src-tauri/
cd gui
pnpm install
pnpm tauri dev
pnpm tauri build
```

The built app will be in `gui/src-tauri/target/release/bundle/`.

### GPU builds

GPU encoding and preview decode need Grok's accelerator plugin, a commercial product sold separately by Grok Image Compression. The released builds encode on the CPU. This section is for a machine that has the plugin.

Grok looks for `libgrokj2k_plugin` in the directory `GRK_PLUGIN_PATH` names, then in the working directory, then beside the executable. It does not search `LD_LIBRARY_PATH` or `PATH` for the plugin. The codec library itself still needs the loader path:

```bash
# rebuilds the GUI against that grok and launches it with the plugin on the
# loader path. Linux looks in lib64/ then lib/ for libgrokj2k_plugin.so,
# macOS in lib/ then lib64/ for the dylib plus grok_kernels.metallib, Windows
# in bin/ then lib/ for grokj2k_plugin.dll.
python3 run-gpu-gui.py /path/to/grok/install

export LD_LIBRARY_PATH=/path/to/grok/lib64
export GRK_PLUGIN_PATH=/path/to/grok/lib64
imfwizard --gpu create --title "My Film" --video master.mov --output ./imp
```

The desktop GPU setting applies to encode and preview decode. Its job log, written beside the package as `<output>.log`, prints `Accelerator: requested, active` and `[ENCODE] Frames on the device: N of M` when the plugin ran. Use `--no-gpu` to override a saved GPU preference for one CLI run. `--threads N` sets the encoder threads, and the same count sizes the accelerator plugin's host threads. The desktop *Encode threads* setting under the GPU checkbox is saved as `encodeThreads` and applies to the GUI and to a CLI run without `--threads`. 0, an empty field, or no flag and no saved setting runs one thread per available CPU. The desktop job log prints the count as `Encode threads: N`.

**GPU encoding on Fedora.** An rpm with the CUDA plugin is built from a local Grok installation that carries it:

```bash
./scripts/build-fedora-rpm.sh /path/to/grok/install
```

The RPM is written under `gui/src-tauri/target/release/bundle/rpm`. The plugin is built for one CUDA compute capability, and the file name carries it in the release field, for example `IMF-Wizard-1.4.0-1.sm75.x86_64.rpm` for a 2080 Ti. Remove an installed test build with `sudo dnf remove imf-wizard`.

That rpm needs two more things on the target machine: the RPM Fusion NVIDIA driver, and a Grok licence entered under Settings. The CUDA runtime is linked into the plugin, so the CUDA toolkit and the NVIDIA Container Toolkit are not needed:

```bash
sudo dnf install akmod-nvidia xorg-x11-drv-nvidia-cuda
sudo akmods --force
sudo reboot
```

**GPU encoding on Ubuntu.** A deb with the CUDA plugin is built inside an Ubuntu 24.04 container with podman, from a Grok source checkout that has the plugin submodule, for one CUDA compute capability:

```bash
./scripts/build-ubuntu-deb.sh /path/to/grok/source 86
```

The deb is written under `gui/src-tauri/target/release/bundle/deb`, for example `IMF-Wizard_1.4.0-sm86_amd64.deb` for an RTX 30 series card, and the script ends by installing it in a plain `ubuntu:24.04` container. Build caches stay in `~/.cache/postperfection/ubuntu24`. Like the rpm, the target machine needs the NVIDIA driver and a Grok licence entered under Settings.

**GPU encoding on macOS.** A dmg with the Metal plugin is built on a Mac from a local Grok installation that carries it:

```bash
./scripts/build-macos-dmg.sh /path/to/grok/install
```

The dmg is written under `gui/src-tauri/target/release/bundle/dmg`, and its name carries `metal`, for example `IMF-Wizard-1.4.0-metal_aarch64.dmg`. Enter a Grok licence under Settings to encode on the GPU.

## Usage

### Create an IMP from J2K + WAV

```bash
imfwizard create \
  --title "My Feature Film" \
  --video /path/to/j2k_frames/ \
  --audio /path/to/audio.wav \
  --output /path/to/output_imp/ \
  --fps-num 24 --fps-den 1
```

`--fps-num`/`--fps-den` take any rate. A video file left without them is encoded
and declared at the rate ffprobe reads from it, and a named rate it does not play
at is refused before the encode. Image sequences, codestream directories and held
stills take the flags, 24/1 without them. The GUI's Frame Rate menu offers
23.976, 24, 25, 29.97, 30, 48, 50, 59.94, 60, 100, 119.88 and 120.

### Set the picture bitrate

```bash
# --bitrate is the J2K target in Mbps, above 0 and at most 1000. It wins over the
# bitrate --profile carries. Without either, the encode runs at a 10:1 ratio.
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --bitrate 100 \
  --output /path/to/output/
```

### Encode to a quality target instead of a ratio

```bash
# --quality-psnr is a PSNR target in dB, at least 20 and at most 80. The encoder
# allocates to that quality rather than to a compression ratio, and the bitrate
# (from --bitrate or --profile) becomes a per-frame byte cap no frame may exceed.
# A frame the quality target pushes over the cap is encoded again by ratio to fit.
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --bitrate 100 \
  --quality-psnr 45 \
  --output /path/to/output/
```

### Create an IMP with subtitles

```bash
# --subtitle takes TTML or IMSC as is, and converts SRT, SCC, ASS/SSA, FCPXML or
# MKS to IMSC on the way in. --subtitle-lang sets the converted document's xml:lang.
imfwizard create \
  --title "My Film" \
  --video /path/to/j2k_frames/ \
  --audio /path/to/audio.wav \
  --subtitle /path/to/subs.ttml \
  --output /path/to/output/
```

### Tag the audio language and apply a delivery preset

```bash
# --audio-lang writes an RFC 5646 LocaleList/Language in the CPL (ST 2067-3),
# and the same tag onto the sound MXF's MCA soundfield group (und without it).
# --audio-title-version is MCATitleVersion on that group, default Original Version.
# --audio-content-kind is MCAAudioContentKind, default PRM, a primary mix.
# --audio-element-kind is MCAAudioElementKind, default FCMP, a final complete mix.
# --profile maps a delivery preset's target bitrate to the J2K compression ratio.
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --audio /path/to/de.wav --audio-lang de-DE \
  --audio-title-version "Original Version" \
  --audio-content-kind PRM \
  --audio-element-kind FCMP \
  --profile netflix \
  --output /path/to/output/
```

### Package an accessibility audio track (AD/HI)

```bash
# --audio-role ad (audio description / visually impaired) or hi (hearing impaired)
# emits an MCA EssenceDescriptor (SoundfieldGroup + chVIN/chHI + RFC 5646 language)
# linked to the audio resource via SourceEncoding (ST 2067-2/-3, XSD-validated).
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --audio /path/to/ad.wav --audio-lang en-US --audio-role ad \
  --output /path/to/output/
```

### Package a Dolby Atmos IAB track (ST 2067-201)

```bash
# --atmos names a directory of IA bitstream frame files, one per picture frame,
# read in name order. They are wrapped unencrypted as an AS-02 IAB track file,
# which the CPL plays as an IABSequence beside the picture and the sound.
# --check refuses a missing or empty directory, and a frame count that differs
# from the picture's. The track's IAB soundfield label takes --audio-lang (und
# without it), the title and the --audio-* soundfield group values.
imfwizard create \
  --title "My Film" \
  --video /path/to/j2k_frames/ \
  --audio /path/to/audio.wav --audio-lang en-US \
  --atmos /path/to/ia_bitstream_frames/ \
  --output /path/to/output/
```

The IAB track file declares a 48 kHz audio sample rate whatever the frames carry.
The GUI does not offer `--atmos` yet, and nothing here converts an ADM master to
IAB: `atmos` below carries ADM as PCM.

### Package HDR/WCG picture (ST 2067-21)

```bash
# --hdr sets the transfer characteristic + colour primaries (pq-bt2020, pq-p3d65 or
# hlg-bt2020), written onto the picture MXF RGBA descriptor and the matching CPL
# EssenceDescriptor.
# --mastering-display (optional, requires --hdr) adds the ST 2086 block. The string is
# the x265 master-display format: G,B,R,WP in 0.00002 units, L(max,min) in 0.0001 cd/m^2.
imfwizard create \
  --title "My Film" \
  --video /path/to/j2k_dir \
  --hdr pq-bt2020 \
  --mastering-display "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(40000000,50)" \
  --max-cll 993 \
  --max-fall 362 \
  --output /path/to/output/
```

`--max-cll` and `--max-fall` take nits (0-65535) and require `--hdr`. ST 2067-21 carries
them as CPL ExtensionProperties, not as MXF descriptor metadata, so they go in the CPL
next to ApplicationIdentification and nowhere else. Clause 7.5 defines both for the PQ
colour systems only, so either flag with `hlg-bt2020` is refused. A Dolby Vision profile
8.1 source fills them from its RPU when neither flag is given, and profile 5 is refused
by name.

Every CPL claims the 2020 edition of ST 2067-21 in its ApplicationIdentification,
`http://www.smpte-ra.org/ns/2067-21/2020`, because the 2016 edition has no COLOR.8 and no
full range Rec.709 RGB. The GUI takes the same three presets in the
Properties panel's HDR control.

The picture's own signalling has to agree with the preset. A source tagged PQ
(`smpte2084`) under `hlg-bt2020`, or one tagged HLG (`arib-std-b67`) under a PQ preset, is
refused naming both, and an HDR source (either tag, or a Dolby Vision RPU) packaged
without `--hdr` is refused naming the preset to pass rather than written as Rec.709 SDR. A
source with no transfer tag says nothing about its colour, so the preset is taken at its
word.

The CLI writes one CPL per `create`. The GUI packages multiple compositions
(one CPL tab each) into a single IMP that shares one PKL and ASSETMAP.

### Crop and fit the picture into a legal raster

```bash
# --auto-crop measures the black borders. --fill-crop would crop to the target
# aspect instead of letterboxing back onto it. --raster takes App 2E rasters only.
imfwizard create \
  --title "My Film" \
  --video /path/to/letterboxed.mov \
  --auto-crop --rotate 90 --raster 1920x1080 \
  --output /path/to/output/
```

### Burn subtitles into the picture

```bash
# Sizes are percents: --burn-font-size and --burn-margin of the frame height,
# --burn-outline-width of the text height. --burn-line-height is a multiple of
# the text height. Colours are RRGGBB or RRGGBBAA. Left out, each keeps its
# default: white text 1/22 of the frame height, 4.55%, with a black shadow,
# 1.25 line height, 8% margin.
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --burn-subtitle /path/to/subs.srt \
  --burn-font-size 8 --burn-colour FFFF00 \
  --burn-effect outline --burn-effect-colour 000000 --burn-outline-width 6 \
  --burn-line-height 1.4 --burn-margin 10 \
  --output /path/to/output/
```

### Route and mix the audio channels

```bash
# Stereo into a three-lane file: left and right straight through, plus a centre
# fed from the left at -6 dB. OUT is a channel name or a 1-based number.
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --audio /path/to/stereo.wav \
  --audio-map "1:L,2:R,1:C@-6" \
  --output /path/to/output/
```

### Check a job before building it

```bash
# Runs the refusals and the hints, encodes nothing, writes nothing.
# Exits 1 on a refusal, 0 otherwise.
imfwizard create --check \
  --title "My Film" \
  --video /path/to/video.mov \
  --audio /path/to/sound.wav \
  --burn-subtitle /path/to/cues.srt \
  --output /path/to/output/
```

### Create an IMP from non-J2K images (auto-encode)

```bash
# A directory of DPX, TIFF, EXR, BMP, JPEG or PNG frames is encoded to J2K on the way in.
# The frames carry no rate of their own, so --fps-num/--fps-den is the rate.
imfwizard create \
  --title "My Film" \
  --video /path/to/dpx_frames/ \
  --audio /path/to/audio.wav \
  --fps-num 24 --fps-den 1 \
  --output /path/to/output/
```

A directory holding anything else is refused by name. TIFF frames are read
directly, at 8, 12 or 16 bits a sample, and every other format decodes through
ffmpeg on the way to the encoder.

### Transcode via ffmpeg

```bash
imfwizard transcode \
  -i input.mov \
  -o output.mov \
  -c prores_ks
```

### Encode image sequence to JPEG 2000

```bash
imfwizard encode \
  -i /path/to/tiff_frames/ \
  -o /path/to/j2k_output/ \
  --bitrate 250
```

### Measure loudness

```bash
imfwizard loudness /path/to/audio.wav
```

Adjust a WAV to a target integrated loudness (clip-safe: refuses and writes
nothing if the gain would push true peak above the ceiling, default -1 dBTP):

```bash
imfwizard loudness in.wav --adjust-to -24 -o out.wav
# raise the ceiling if you accept the reported headroom
imfwizard loudness in.wav --adjust-to -24 --true-peak -0.5 -o out.wav
```

### Convert subtitles to IMSC/TTML

```bash
# SRT/SCC flatten to text; ASS/SSA, FCPXML, and MKS keep styling and placement
imfwizard subtitle-convert -i subs.ass -o subs.ttml

# --font-size (percent of the frame height) and --colour (RRGGBB or RRGGBBAA)
# are written as the document's default style, which a run carrying its own
# colour still overrides. The size goes out as a cell-relative tts:fontSize
# against a declared ttp:cellResolution, since a bare TTML percentage is read
# against the parent element's size, not the frame. Authored TTML is copied
# unchanged, so asking for either alongside a .ttml input is refused.
imfwizard subtitle-convert -i subs.srt -o subs.ttml --font-size 6 --colour FFFF00
```

### Supplemental IMP (OV + supplemental)

Package only the new or changed track files against an existing OV. The new CPL
references the OV's unchanged track files by their UUIDs (present in the OV, not
duplicated); the supplemental's ASSETMAP/PKL list only the files physically here.
Track selector is `<path>@<track>` where track is `video`, `audio[:N]`, or
`subtitle[:N]` (N is the 0-based track index within that kind).

```bash
# replace the OV audio with a French dub, keep the OV video by reference
imfwizard supplement --ov /path/to/OV --title "French Dub" -o /path/to/SUPP \
  --replace french_dub.wav@audio

# add a new subtitle track and replace the second audio track
imfwizard supplement --ov /path/to/OV --title "v2" -o /path/to/SUPP \
  --add subs_de.ttml@subtitle --replace commentary.wav@audio:1
```

Video input is a J2K codestream directory; audio is WAV; subtitle is TTML/IMSC.
Deliver the supplemental alongside its OV: a
validator resolves the CPL's OV references against the OV's ASSETMAP, so
validating the supplemental on its own reports the OV track files as missing.

### Validate an IMP

```bash
# create runs this itself over every package it writes
imfwizard validate /path/to/imp/

# Also validate the CPL, PKL, AssetMap and any OPL against the SMPTE schemas the binary carries.
# rust/crates/imfwizard-core/schemas/README.md lists their sources and licences
imfwizard validate /path/to/imp/ --xsd
# Or against the schemas in a directory (else IMF_SCHEMA_DIR). A document with no schema there prints SKIPPED
imfwizard validate /path/to/imp/ --xsd --schema-dir /path/to/st2067-xsds

# Also run Netflix Photon (needs a JRE plus Photon and its dependencies).
# Netflix ships no fat jar, so fetch the jars into one directory and point at it:
scripts/fetch_photon.sh ~/.cache/imfwizard/photon
imfwizard validate /path/to/imp/ --photon --photon-jar ~/.cache/imfwizard/photon
# or set PHOTON_JAR=~/.cache/imfwizard/photon
```

### Display IMP info

```bash
imfwizard info /path/to/existing_imp/
```

### List delivery presets

```bash
# Apply one to an encode with `create --profile <name>` (maps target bitrate).
imfwizard profiles
```

### Encode to ProRes

```bash
imfwizard prores \
  -i /path/to/master.mov \
  -o /path/to/master_prores.mov \
  -p hq

# A directory of numbered frames needs --fps-num and --fps-den, stills carry no rate
imfwizard prores -i /path/to/frames/ -o /path/to/master_prores.mov --fps-num 24 --fps-den 1

# An IMP directory exports its first CPL's picture and main sound as ProRes 4444
# at the CPL's edit rate. --cpl <uuid> picks another composition. --ov <dir>
# names the OV a supplemental IMP was built against, so the track files its CPL
# references but does not carry are resolved through the OV's ASSETMAP.
imfwizard prores \
  -i /path/to/imp/ \
  -o /path/to/delivery.mov \
  --container 4k-scope

# Containers: 2k-scope (2048×858), 2k-flat (1998×1080), 2k-full (2048×1080),
#             4k-scope (4096×1716), 4k-flat (3996×2160), 4k-full (4096×2160)
# The picture keeps its aspect ratio and is padded with black. Without
# --container it keeps its own raster. An unknown name is refused.
# No file inside the IMP is written.
```

### Burn subtitles into video

```bash
imfwizard burn-in \
  -i /path/to/video.mp4 \
  -s /path/to/subs.srt \
  -o /path/to/output_burned.mp4
```

### Bitrate analytics

```bash
# Track counts and package size
imfwizard analytics -d /path/to/imp/

# JSON output for dashboards
imfwizard analytics -d /path/to/imp/ --json

# Per-second bitrate, min, max, mean, spread and a --histogram-buckets histogram (default 20)
imfwizard analytics -d /path/to/imp/ --video /path/to/imp/VIDEO_<uuid>.mxf --json
```

### REST API server

```bash
# Start on host:port, optionally requiring an API key
imfwizard serve --bind 0.0.0.0:9090 --api-key "my-secret"

# Endpoints:
#   GET  /api/v1/health       , health check
#   POST /api/v1/create       , submit IMP creation job
#   POST /api/v1/validate     , submit validation job
#   POST /api/v1/encode       , submit encoding job
#   POST /api/v1/transcode    , submit transcode job
#   GET  /api/v1/jobs         , list all jobs
#   GET  /api/v1/jobs/<id>    , job status
#   DELETE /api/v1/jobs/<id>  , cancel job
#   GET  /api/v1/profiles     , list delivery presets
#   GET  /api/v1/tools        , dependency check
#   POST /api/v1/pause        , refuse new submissions
#   POST /api/v1/resume       , accept submissions again
#   GET  /metrics             , Prometheus metrics
```

Every job submission takes the same JSON object. `input` and `output` are
paths on the server, `title` is the ContentTitle a `create` job writes into its
CPL and the description the other job types carry. Any other field is a 400
naming it, and a body that is not a JSON object is a 400 too:

```bash
curl -X POST http://localhost:9090/api/v1/create \
  -H "X-Api-Key: my-secret" \
  -H "Content-Type: application/json" \
  -d '{"input":"/masters/feature/j2k","output":"/deliveries/feature","title":"My Feature"}'
# {"id":1,"status":"queued"}

curl -H "X-Api-Key: my-secret" http://localhost:9090/api/v1/jobs/1
# {"id":1,"job_type":"Create","state":"Running","progress":0.0, ...}
```

A `create` job takes a directory of JPEG 2000 codestreams as its `input`, a
`validate` job takes an IMP directory, and `encode` and `transcode` take the
file or frame directory their CLI commands take. These four are the only job
types the server has a route for. The queue's `Qc` and `Copy` job types fail
with an error when one runs, and the CLI has no `qc` or `copy` command. Write
a QC report with `imfwizard report` (alias `qc-report`).

`--api-key` is required on every endpoint but `/api/v1/health` and `/health`,
in `X-Api-Key` or `Authorization: Bearer`, `/metrics` included. Without the flag
nothing is required and the whole API is open, so bind it to a network you
trust.

`POST /api/v1/pause` refuses new submissions with 503 until `POST
/api/v1/resume`. A job already queued or running is untouched, so the worker
drains what it has; cancel those with `DELETE /api/v1/jobs/<id>`.

### Dependency check (doctor)

```bash
# Check external tool availability
imfwizard doctor

# JSON output for CI/CD or scripting
imfwizard doctor --json
```

### EDL conform

```bash
# Print what a CMX 3600 EDL or an FCP7 XML timeline says, writing nothing
imfwizard conform -i timeline.edl
imfwizard conform -i project.xml --json

# Build an IMP that follows the timeline
imfwizard conform -i timeline.edl --media-dir /path/to/rushes -o /path/to/imp
```

Each event's reel name is matched against the file names under `--media-dir`,
and its source in and out are read as frame counts from the start of that file.
Every event becomes one picture track file encoded from its own source range and
one sound track file cut from the same media, and the CPL plays them in record
order through a single main image track and a single main audio track. Every
event's media has to carry the same raster, and either all of them carry sound
or none does. `conform_manifest.json` in the output directory records the plan
and the track file each event became.

### Frame comparison

```bash
# Per-frame PSNR/SSIM of the picture track file each IMP's first CPL plays
imfwizard compare -a imp_v1/ -b imp_v2/ --pixel --json

# The same on two video or MXF files
imfwizard compare -a /path/to/imp_v1/VIDEO_<uuid>.mxf -b /path/to/imp_v2/VIDEO_<uuid>.mxf --pixel --json

# VMAF score (needs an ffmpeg built with libvmaf); combine with --pixel and --json
imfwizard compare -a reference.mxf -b encoded.mxf --vmaf --json
```

### Dolby Atmos import

```bash
imfwizard atmos -i atmos_master.bwf -o output_dir/
```

This wraps the ADM master's channels as PCM and writes the ADM XML beside it. An
IAB track comes from `create --atmos` instead.

### MCA label generation

Writes SMPTE 377-4 MCA labels into a sound MXF's descriptor, rewrapping the PCM
under the same asset id. `--layout` takes `mono`, `stereo`, `51` or `71` and has
to match the channel count the file carries. When the MXF sits in an IMP, the
CPL's essence descriptor and the PKL hashes are rewritten with it, so the package
still validates.

```bash
# 5.1 surround, spoken French
imfwizard mca -i audio.mxf -l 51 -L fr-CA

# 7.1 surround
imfwizard mca -i audio.mxf -l 71 -L en
```

### Audio description mixing

```bash
imfwizard audio-desc -i mix_51.wav --narration ad_narration.wav -o combined.wav --duck-level -12
```

### Apply 3D LUT

```bash
# A directory of numbered frames in, frame_000001 onward out in the same image format
imfwizard lut --lut grading.cube -i /frames/ -o /graded_frames/ --fps-num 24 --fps-den 1

# The same with ffmpeg image patterns
imfwizard lut --lut grading.cube -i /frames/frame_%06d.tif -o /graded_frames/frame_%06d.tif
```

### ACES conversion

```bash
# AP0 to Rec.709 primaries and transfer, no ACES rendering transform, TIFF out.
# A directory of numbered frames in, frame_000001.tif onward out
imfwizard aces -i /ap0_frames/ -o /rec709_frames/ --fps-num 24 --fps-den 1

# The same with ffmpeg image patterns
imfwizard aces -i /ap0_frames/frame_%06d.tif -o /rec709_frames/frame_%06d.tif
```

### A/V sync check

```bash
# Reports initial A/V offset plus drift accumulated across the program
imfwizard av-sync -i /path/to/video.mxf
```

### Platform compliance

```bash
# Check Netflix compliance (standards: smpte, netflix, disney, hbo, dolby, dci-2k, dci-4k, archival, broadcast)
imfwizard compliance -i /path/to/imp/ -s netflix
```

### CPL annotation

```bash
imfwizard annotate -i /path/to/imp/ -t "Color correction pass 2"
```

### Partial version

```bash
# Copy the files a given CPL UUID references into a new IMP
imfwizard partial-version -i /orig_imp/ -o /partial/ --cpl <cpl-uuid>
```

### Slate

```bash
# Prepend black frames with white centred text, --frames defaults to 24.
# A directory of numbered frames in, frame_000001 onward out in the same image format
imfwizard slate -i /frames/ -o /slated/ --text "MY FILM, Final Master" --frames 48 --fps-num 24 --fps-den 1

# A video in, frames out through an image pattern
imfwizard slate -i /path/to/clip.mov -o /slated/slated_%04d.png --text "MY FILM, Final Master" --frames 48
```

### Video retiming

```bash
# Retime a video file to 25 fps via ffmpeg
imfwizard retime -i input.mov -o output_25.mov -f 25
```

## Architecture

```
imfwizard/
├── rust/                # Rust workspace
│   ├── crates/
│   │   ├── imfwizard-core/  # Core library, packaging, encoding, tools, REST API, Atmos
│   │   └── imfwizard-cli/   # CLI binary (imfwizard)
│   └── Cargo.toml
├── gui/                 # Tauri 2 desktop application
│   ├── src/             # Frontend (Vite + vanilla JS)
│   └── src-tauri/       # Rust backend: job queue, preview and package commands over imfwizard-core, postkit and guikit
└── docs/                # GitHub Pages site
```

IMF Wizard shares common functionality with [DCP Wizard](https://github.com/PostPerfection/dcpwizard)
via the [postkit](https://github.com/PostPerfection/postkit) library (encoding, hashing, job queue,
preferences, REST API, the watch folder loop, the cancellable ffmpeg transcode runner behind
`transcode`, and more). The IMP build each watched master starts stays in IMF Wizard.

## License

AGPL-3.0-or-later. Copyright (C) 2026 Grok Image Compression Inc. See [LICENSE](LICENSE).
