# IMF Wizard

[![CI](https://github.com/PostPerfection/imfwizard/actions/workflows/ci.yml/badge.svg)](https://github.com/PostPerfection/imfwizard/actions/workflows/ci.yml)

[Documentation](https://postperfection.github.io/imfwizard/)

Interoperable Master Format (IMF) package creator, CLI tool and desktop GUI. Written in Rust.

Version 1.1 writes complete CPL, PKL, and ASSETMAP references, uses base64 package hashes, identifies App 2E, and rejects incompatible picture essence before packaging.

## Overview

IMF Wizard creates valid IMF packages (Interoperable Master Packages) from
video sources, image sequences, and WAV audio, conforming to SMPTE ST 2067 (App#2E).

## Features

### Packaging & Wrapping
- **Original Version IMP creation** from J2K + WAV
- **TTML / IMSC subtitle** packaging as AS-02 timed text MXF
- **Subtitle conversion** to IMSC/TTML from SRT, SCC (CEA-608 pop-on captions), ASS/SSA, FCPXML, and MKS (Matroska); ASS/FCPXML/MKS keep styling and placement (italic/bold/underline/colour, alignment, position) in the TTML output
- **AS-02 MXF wrapping** (SMPTE 2067-5), CPL/PKL/AssetMap generation
- **SHA-1 hashing** for PKL/ASSETMAP asset integrity
- **Optional XML-DSIG signing** of CPL/PKL/ASSETMAP (`sign` / `verify-sig`, needs a cert + key)
- **IMF to DCP**, rewrap a single-composition IMP (one picture, optional one sound) to a DCP

### Encoding & Transcoding
- **Image encoding pipeline**, DPX, TIFF, EXR, PNG, BMP, JPEG → 12-bit JPEG 2000 via Grok (`grk_compress`)
- **Video transcoding via ffmpeg** (`transcode`, pick the output codec, e.g. libx264/prores)
- **ProRes encoding** (`prores`), encode a video/image sequence to a ProRes .mov master
- **Burn-in during the encode**, `create --burn-subtitle <file>` (+ `--burn-subtitle-font <ttf/otf>`) draws the cues into the picture as it encodes, so a burnt master costs one generation rather than two. Reads SRT, ASS/SSA, SCC, FCPXML and MKS/MKV. Burnt text is part of the image and registers no timed-text track, the same file cannot be both, and burning onto an already-X'Y'Z' source, a J2K directory or a held still is refused
- **Subtitle burn-in as a standalone pass**, `burn-in` renders SRT/TTML into video frames via ffmpeg, outside a package
- **Trim**, `create --trim-start` / `--trim-end` take frames (`48f`) or seconds (`2s`) off the head and tail; picture, sound and timed text move together, and cues outside the kept range are dropped or clamped
- **Still image with duration**, `create --still-length` holds a single image (dpx, tif, exr, png, bmp) for that long, encoding it once and repeating the codestream

### HDR & Advanced
- **HDR/WCG essence metadata (ST 2067-21)** — `create --hdr pq-bt2020|pq-p3d65` writes the transfer/colour ULs onto the picture MXF RGBA descriptor and the CPL EssenceDescriptor. Optional `--mastering-display` adds the ST 2086 block, and `--max-cll` / `--max-fall` add the content light levels as CPL ExtensionProperties
- **Dolby Vision** RPU metadata injection (via dovi_tool)
- **HDR10+ dynamic metadata** injection, re-encodes with libx265 to write SEI (via hdr10plus_tool)
- **Dolby Atmos / immersive audio packaging** (ADM channels carried as PCM MXF; not re-encoded to a Dolby IAB bitstream)

### Quality Control
- **Loudness analysis**, EBU R128 integrated/true-peak measurement (measure only, no normalization)
- **Native XSD schema validation**, validate CPL/PKL/AssetMap XML against SMPTE ST 2067 XSD schemas (via xmllint)
- **Structural validation** via dcpdoctor-core (ASSETMAP/PKL/hash checks) plus CPL/PKL signature verification
- **Netflix Photon validation** (optional), gated behind `validate --photon` (needs a JRE + Photon jar)
- **PSNR / SSIM** frame comparison between two image sequences
- **VMAF** (optional) via `compare --vmaf` (needs an ffmpeg built with libvmaf)
- **Bitrate analytics**, per-second throughput, histogram, standard deviation (JSON output for dashboards)
- **QC report** generation (text / JSON / HTML)
- **Platform compliance checking** (ffprobe-based) against Netflix, Dolby, Amazon, SMPTE profiles

### Color & Audio Processing
- **Source colour space**, `create --source-colourspace rec709|xyz` picks whether the encoder runs its X'Y'Z' transform (rec709, the default) or leaves frames that already carry it alone (xyz). p3, rec2020, aces, acescg and logc are accepted spellings but refused, since nothing here builds the transform they would need
- **Audio delay**, `create --audio-delay <ms>` shifts the sound against the picture without changing the running time, padding one end and truncating the other
- **3D LUT application**, apply .cube LUTs to image sequences via ffmpeg lut3d
- **ACES pipeline**, full IDT→RRT→ODT pipeline via ctlrender (with ffmpeg fallback)
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
- **REST API server**, HTTP interface for /create, /validate, /encode, /transcode, /jobs, /tools, /pause, /resume (in-memory queue with a background worker; jobs live for the server process only)
- **EDL/FCP XML import**, parse CMX 3600 EDL and Final Cut Pro 7 XML timelines
- **SDI preview (Blackmagic DeckLink)**, play J2K frames via mpv DeckLink output
- **Dependency management (`doctor`)**, check external tool dependencies with version detection and JSON output

### Workflow & Automation
- **Delivery presets**, profiles (Netflix, Amazon, Cinema 2K/4K, ...); apply one to an encode with `create --profile <name>`
- **Watch folder**, print filesystem events for a directory
- **EDL conform**, import CMX3600/FCP7 edit decisions to build a CPL timeline
- **S3 / Aspera / rsync upload** of completed IMPs, with a SQLite delivery tracker
- **Partial restore**, extract tracks from existing IMPs back to raw files (asdcp-unwrap)

### Comparison & Analysis
- **IMF package compare** (`compare`), metadata diff of two IMPs (title, CPL count, duration, edit rate) or pixel PSNR/SSIM/VMAF with `--pixel`/`--vmaf`
- **MXF probe**, inspect MXF files and extract frames (via ffmpeg)

### Distributed & Advanced
- **KDM generation**, generate SMPTE 430-1 Key Delivery Messages for encrypted DCP
- **Dolby Vision Profile 8.1**, HDR10-compatible single-layer DV (MEL/FEL mapping, profile 4→8.1 conversion)
- **Prometheus metrics**, `/metrics` endpoint on REST API exposing job-state gauges
- **Shell tab completion**, bash, zsh, and fish completion scripts (`imfwizard completion bash`)

### Desktop GUI (Tauri 2)
- **Dark theme** by default with optional light mode toggle
- **File import** (video, WAV, TTML/subtitle) via file picker; builds package the selected picture, audio, and subtitle
- **Timeline editor**, visual segment arrangement
- **Keyboard shortcuts**, Ctrl+N/O/B/P/I, Ctrl+Shift+S, Ctrl+1..7 tab navigation and Space/arrows/Home during preview. Ctrl+K opens the shortcut list, where clicking a shortcut rebinds it (Backspace clears, Escape cancels) and the rebindings are saved
- **Progress bars**, real-time progress tracking for encode/wrap jobs
- **IMP metadata editor**, edit CPL title/annotation
- **Preview player**, mpv-based playback with timeline scrubber (click-to-seek, drag-to-scrub, timecode display)
- **Subtitle burn-in**, GUI for hardcoding subs into video
- **Job queue manager**, submit, monitor, cancel background jobs
- **Progress notifications**, system notifications when jobs complete
- **Recent projects**, quick access to previously created IMPs

### Packaging & Deployment
- **Docker image**, headless batch processing (`docker run imfwizard create ...`)
- **Flatpak**, Linux desktop distribution via Flathub
- **macOS .dmg**, universal binary with code signing and notarization
- REST API mode with Prometheus-compatible `/metrics` endpoint

### Mastering & Compliance
- **DCDM creation**, Digital Cinema Distribution Master (X'Y'Z' 12/16-bit) as intermediate format
- **Visible watermark burn-in**, burn operator/session text into an image sequence
- **Trailer packaging**, ratings cards (MPAA/BBFC/FSK), green/red band, countdown leaders
- **Content version tracker**, SQLite database tracking version history and delivery destinations
- **Accessibility compliance**, verify AD/HI/SL tracks against CVAA, EAA, AODA, Ofcom standards

## Installation

### Pre-built binaries (recommended)

Download from the [GitHub Releases](https://github.com/PostPerfection/imfwizard/releases/latest) page:

| Platform | CLI | Desktop GUI |
|----------|-----|-------------|
| **Linux** (x86_64) | `imfwizard-linux-x86_64.tar.gz` | `.deb`, `.AppImage` |
| **macOS** (Apple Silicon) | `imfwizard-macos-aarch64.tar.gz` | `.dmg` |
| **Windows** (x86_64) | `imfwizard-windows-x86_64.zip` | `.msi` |

The CLI binary carries everything but the Grok JPEG 2000 codec, which it links dynamically. The Windows zip ships `grokj2k.dll` beside the exe; on Linux and macOS the library comes from the Grok install. Extract and run.

### Install from source

Every build needs the [Grok](https://grok.rocks/) JPEG 2000 codec, since the picture encoder calls it in-process. Build and install it once, then put it on the pkg-config and loader paths:

```bash
git clone --recurse-submodules --branch v20.3.10 https://github.com/GrokImageCompression/grok.git
cmake -S grok -B grok/build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$HOME/bin/grok"
cmake --build grok/build --parallel
cmake --install grok/build

export PKG_CONFIG_PATH="$HOME/bin/grok/lib64/pkgconfig:$HOME/bin/grok/lib/pkgconfig:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="$HOME/bin/grok/lib64:$HOME/bin/grok/lib:$LD_LIBRARY_PATH"
```

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get install -y pkg-config libxml2-dev libssl-dev libxerces-c-dev
# For GUI: also install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev

git clone --recurse-submodules https://github.com/PostPerfection/imfwizard.git
cd imfwizard/rust
cargo build --release
# Binary at rust/target/release/imfwizard
```

#### macOS

```bash
brew install pkg-config libxml2 openssl@3 xerces-c

export OPENSSL_DIR=$(brew --prefix openssl@3)
export PKG_CONFIG_PATH="$(brew --prefix openssl@3)/lib/pkgconfig:$(brew --prefix libxml2)/lib/pkgconfig:$(brew --prefix xerces-c)/lib/pkgconfig"

cd rust
cargo build --release
```

#### Windows

```powershell
# Using vcpkg (recommended)
vcpkg install libxml2 openssl xerces-c --triplet x64-windows

$env:VCPKG_ROOT = "$env:VCPKG_INSTALLATION_ROOT"
$env:CMAKE_TOOLCHAIN_FILE = "$env:VCPKG_INSTALLATION_ROOT/scripts/buildsystems/vcpkg.cmake"

cd rust
cargo build --release
```

### Optional runtime dependencies

| Dependency | Purpose | Install |
|-----------|---------|---------|
| `ffmpeg` / `ffprobe` | Video transcoding, loudness, quality metrics | `apt install ffmpeg` / `brew install ffmpeg` / [ffmpeg.org](https://ffmpeg.org/download.html) |
| `grk_compress` | JPEG 2000 encoding of image sequences (video goes through the linked Grok library instead) | [grok.rocks](https://grok.rocks/) |
| `mpv` | GUI preview player | `apt install mpv` / `brew install mpv` / [mpv.io](https://mpv.io/installation/) |
| `dovi_tool` | Dolby Vision RPU injection | [GitHub](https://github.com/quietvoid/dovi_tool/releases) |
| `hdr10plus_tool` | HDR10+ dynamic metadata | [GitHub](https://github.com/quietvoid/hdr10plus_tool/releases) |
| `ctlrender` | ACES CTL transforms (IDT/RRT/ODT) | [GitHub](https://github.com/ampas/CTL) |
| `xmllint` | XSD schema validation of IMP XML | `apt install libxml2-utils` / `brew install libxml2` |
| ffmpeg with `libvmaf` | VMAF in `compare --vmaf` | ffmpeg built `--enable-libvmaf` (check `ffmpeg -filters \| grep libvmaf`) |
| JRE + Photon jars | `validate --photon` (Netflix Photon) | `apt install default-jre`; then `scripts/fetch_photon.sh` |
| `ascp` | Aspera FASP high-speed transfer | [IBM Aspera](https://www.ibm.com/aspera) |
| AWS CLI | S3 upload | [docs.aws.amazon.com](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html) |

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
cd gui
pnpm install
pnpm tauri dev
pnpm tauri build
```

The built app will be in `gui/src-tauri/target/release/bundle/`.

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

### Create an IMP with subtitles

```bash
imfwizard create \
  --title "My Film" \
  --video /path/to/j2k_frames/ \
  --audio /path/to/audio.wav \
  --subtitle /path/to/subs.ttml \
  --output /path/to/output/
```

### Tag the audio language and apply a delivery preset

```bash
# --audio-lang writes an RFC 5646 LocaleList/Language in the CPL (ST 2067-3).
# --profile maps a delivery preset's target bitrate to the J2K compression ratio.
imfwizard create \
  --title "My Film" \
  --video /path/to/video.mov \
  --audio /path/to/de.wav --audio-lang de-DE \
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

### Package HDR/WCG picture (ST 2067-21)

```bash
# --hdr sets the transfer characteristic + colour primaries (pq-bt2020 or pq-p3d65),
# written onto the picture MXF RGBA descriptor and the matching CPL EssenceDescriptor.
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
next to ApplicationIdentification and nowhere else.

The CLI writes one CPL per `create`. The GUI packages multiple compositions
(one CPL tab each) into a single IMP that shares one PKL and ASSETMAP.

### Create an IMP from non-J2K images (auto-encode)

```bash
# Input can be DPX, TIFF, EXR, PNG, automatically encoded to J2K
imfwizard create \
  --title "My Film" \
  --video /path/to/dpx_frames/ \
  --audio /path/to/audio.wav \
  --output /path/to/output/
```

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

Video input is a J2K codestream directory; audio is WAV; subtitle is TTML/IMSC
(same essence inputs as `create`). Deliver the supplemental alongside its OV: a
validator resolves the CPL's OV references against the OV's ASSETMAP, so
validating the supplemental on its own reports the OV track files as missing.

### Validate an IMP

```bash
imfwizard validate /path/to/imp/

# Also validate XML against the SMPTE ST 2067 schemas
imfwizard validate /path/to/imp/ --xsd

# Also run Netflix Photon (needs a JRE plus Photon and its dependencies).
# Netflix ships no fat jar, so fetch the jars into one directory and point at it:
scripts/fetch_photon.sh ~/.cache/imfwizard/photon
imfwizard validate /path/to/imp/ --photon --photon-jar ~/.cache/imfwizard/photon
# or set PHOTON_JAR=~/.cache/imfwizard/photon
# Set PHOTON_DIR to the same directory as well: plain `validate` runs a second
# Photon pass via dcpdoctor, which otherwise clones and gradle-builds Photon.
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
```

### Burn subtitles into video

```bash
imfwizard burn-in \
  -i /path/to/video.mp4 \
  -s /path/to/subs.srt \
  -o /path/to/output_burned.mp4
```

### Scale/crop an IMP to a target resolution

```bash
# Rewrap an IMP's essence to a 4K-scope ProRes .mov master
imfwizard target-convert \
  -i /path/to/imp/ \
  -o /path/to/delivery/ \
  -t 4k-scope

# Targets: 2k-scope (2048×858), 2k-flat (1998×1080), 2k-full (2048×1080),
#          4k-scope (4096×1716), 4k-flat (3996×2160), 4k-full (4096×2160)
# An unknown target errors instead of falling back to 1080p.
```

### Bitrate analytics

```bash
# Human-readable summary
imfwizard analytics -d /path/to/imp/

# JSON output for dashboards
imfwizard analytics -d /path/to/imp/ --json
```

### REST API server

```bash
# Start on host:port, optionally requiring an API key
imfwizard rest-api --bind 0.0.0.0:9090 --api-key "my-secret"

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
#   POST /api/v1/pause        , pause job queue
#   POST /api/v1/resume       , resume job queue
#   GET  /metrics             , Prometheus metrics
```

### Dependency check (doctor)

```bash
# Check external tool availability
imfwizard doctor

# JSON output for CI/CD or scripting
imfwizard doctor --json
```

### EDL import

```bash
# Parse a CMX 3600 EDL
imfwizard edl-import -i timeline.edl

# Parse Final Cut Pro XML
imfwizard edl-import -i project.fcpxml
```

### Frame comparison

```bash
# Compare two IMPs or video files (add --pixel for per-frame PSNR/SSIM on video MXF)
imfwizard compare -a /path/to/imp_v1/ -b /path/to/imp_v2/ --pixel --json

# VMAF score (needs an ffmpeg built with libvmaf); combine with --pixel and --json
imfwizard compare -a reference.mxf -b encoded.mxf --vmaf --json
```

### Dolby Atmos import

```bash
imfwizard atmos -i atmos_master.bwf -o output_dir/
```

### MCA label generation

```bash
# Inject 5.1 surround MCA labels into an audio MXF's CPL
imfwizard mca -i audio.mxf -l 51 -L en

# 7.1 surround
imfwizard mca -i audio.mxf -l 71 -L en
```

### Audio description mixing

```bash
imfwizard audio-desc -i mix_51.wav --narration ad_narration.wav -o combined.wav --duck-level -12
```

### Apply 3D LUT

```bash
imfwizard lut --lut grading.cube -i /frames/ -o /graded_frames/
```

### ACES color pipeline

```bash
imfwizard aces -i /log_frames/ -o /aces_frames/ --idt ARRI_LogC4 --odt P3D65_PQ_1000nits
```

### A/V sync check

```bash
# Reports initial A/V offset plus drift accumulated across the program
imfwizard av-sync -i /path/to/video.mxf
```

### Platform compliance

```bash
# Check Netflix compliance (standards: smpte, netflix, dolby, amazon)
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
# Prepend a black text slate as image frames
imfwizard slate -i /frames/ -o /slated/ --text "MY FILM, Final Master" --frames 48
```

### Video retiming

```bash
# Retime a video file to 25 fps via ffmpeg
imfwizard retime -i input.mov -o output_25.mov -f 25
```

### SDI output (Blackmagic DeckLink)

```bash
# Play an IMP or MXF via mpv DeckLink output on device 0
imfwizard sdi-preview -i /path/to/imp/ -d 0
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
│   └── src-tauri/       # Rust backend (plugin shell)
└── docs/                # GitHub Pages site
```

IMF Wizard shares common functionality with [DCP Wizard](https://github.com/PostPerfection/dcpwizard)
via the [postkit](https://github.com/PostPerfection/postkit) library (encoding, transcoding, hashing,
job queue, preferences, REST API, watch folders, and more).

## License

AGPL-3.0-or-later. Copyright (C) 2026 Grok Image Compression Inc. See [LICENSE](LICENSE).
