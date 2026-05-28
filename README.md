# IMF Wizard

[![CI](https://github.com/PostPerfection/imfwizard/actions/workflows/ci.yml/badge.svg)](https://github.com/PostPerfection/imfwizard/actions/workflows/ci.yml)

[Documentation](https://postperfection.github.io/imfwizard/)

Interoperable Master Format (IMF) package creator — CLI tool and desktop GUI. Written in Rust.

## Overview

IMF Wizard creates valid IMF packages (Interoperable Master Packages) from
video sources, image sequences, and WAV audio, conforming to SMPTE ST 2067 (App#2E).

## Features

### Packaging & Wrapping
- **Original Version IMP creation** from J2K + WAV
- **Supplemental IMP** creation with segment replacement
- **Multi-CPL IMP** — multiple compositions sharing a track pool
- **TTML / IMSC subtitle** packaging as AS-02 timed text MXF
- **Closed captions** — SCC (CEA-608) and SRT → TTML conversion
- **Accessibility tracks** — Audio Description, Hearing Impaired, Sign Language, Commentary
- **Multi-language audio** with RFC 5646 language tags and MCA labels
- **AS-02 MXF wrapping** (SMPTE 2067-5), CPL/PKL/AssetMap generation
- **SHA-1 hashing** and optional **XML-DSIG signing**

### Encoding & Transcoding
- **Image encoding pipeline** — DPX, TIFF, EXR, PNG, BMP, JPEG → JPEG 2000 (via grok, GPU and CPU)
- **ProRes / DNxHR / H.264 / H.265 transcoding** — video files → image sequence → J2K (via ffmpeg)
- **ProRes IMF packaging** — create App#2E IMPs directly from ProRes .mov files
- **ACES (App#5) colour management** — ACES transforms during encode
- **Scale / crop / letterbox** options for resolution adaptation
- **Subtitle burn-in** — permanently render SRT/TTML/SCC into video frames (for festivals)

### HDR & Advanced
- **Dolby Vision 4.0** RPU metadata injection (via dovi_tool)
- **HDR10+ dynamic metadata** injection (via hdr10plus_tool)
- **HDR/WCG color metadata** — ST 2067-21 (PQ, HLG, BT.2020, P3-D65)
- **IAB / Dolby Atmos** immersive audio packaging

### Quality Control
- **Loudness analysis** — EBU R128 / ATSC A/85 measurement and normalization
- **Photon validation** — validate output IMPs using Netflix Photon
- **Native XSD schema validation** — validate CPL/PKL/AssetMap XML against SMPTE ST 2067 XSD schemas (via xmllint)
- **Frame-level QC** — per-frame bitrate analysis with over/under-budget detection
- **VMAF / PSNR / SSIM** quality metrics (via ffmpeg libvmaf)
- **Bitrate analytics** — per-second throughput, histogram, standard deviation (JSON output for dashboards)
- **QC HTML report** generation
- **PDF QC report** — generate PDF QC reports using wkhtmltopdf or weasyprint
- **Frame-accurate comparison** — PSNR/SSIM comparison between two IMPs with diff report
- **Platform compliance checking** — validate against Netflix, Disney+, Amazon, Apple, Cinema, Broadcast specs
- **Detailed QC report** — HTML report with package info, track listing, loudness, thumbnails

### Color & Audio Processing
- **3D LUT application** — apply .cube LUTs to image sequences via ffmpeg lut3d
- **ACES pipeline** — full IDT→RRT→ODT pipeline via ctlrender (with ffmpeg fallback)
- **Audio description mixing** — combine AD narration with main mix using ducking
- **MCA label generation** — SMPTE ST 377-4 Multi-Channel Audio labeling (5.1, 7.1, stereo presets)
- **Dolby Atmos ADM BWF import** — import ADM Broadcast Wave Format to IAB MXF
- **A/V sync detection & repair** — detect audio/video drift and auto-fix with trim/pad

### Versioning & Annotation
- **CPL annotation** — add revision notes and author metadata to CPL XML
- **Partial version creation** — create supplemental IMPs replacing specific reel segments
- **Subtitle retiming** — convert TTML/SRT timing between framerates (24→25, 23.976→24, etc.)

### Pre-roll & Leaders
- **Slate generation** — countdown, SMPTE bars, academy leader, text slate, black (as TIFF image sequence)
- **Reference tone** — optional 1kHz WAV tone paired with visual pre-roll

### Integration & Extensibility
- **REST API server** — HTTP interface for /create, /validate, /encode, /transcode, /jobs
- **Webhook notifications** — POST to external endpoints on job completion/failure (Slack, Teams, CI/CD)
- **EDL/FCP XML import** — parse CMX 3600 EDL and Final Cut Pro XML timelines
- **Plugin system** — discover and execute Python plugin scripts with pre/post hooks
- **SDI output (Blackmagic DeckLink)** — play J2K frames over HD-SDI via GStreamer decklinkvideosink

### Workflow & Automation
- **Delivery presets** — Netflix, Disney+, Amazon, Apple TV+, Cinema 2K/4K, Broadcast, Archival
- **Batch delivery** — create IMPs for multiple platforms in a single pass
- **IMF-to-DCP conversion** — repackage IMF as Digital Cinema Package for theatrical playback
- **Job queue daemon** — background processing with Unix socket / Windows named pipe IPC
- **Watch folder** — auto-IMP creation when files appear
- **EDL/AAF conform** — import CMX3600 edit decisions to auto-build CPL timelines
- **S3 cloud upload** — push completed IMPs to AWS S3
- **Aspera FASP** — high-speed delivery via IBM Aspera
- **Partial restore** — extract tracks/segments from existing IMPs back to raw files

### Comparison & Analysis
- **IMF package diff** — compare two IMPs and show track/segment changes
- **MXF playback/probe** — inspect MXF files, extract frames, generate thumbnails (via GStreamer/ffmpeg)
- **OTIOZ import** — import OpenTimelineIO zip bundles with timeline-to-CPL conversion

### Distributed & Advanced
- **Multi-node render** — distribute J2K encoding across multiple machines (coordinator + worker mode)
- **KDM generation** — generate SMPTE 430-1 Key Delivery Messages for encrypted DCP
- **Dolby Vision Profile 8.1** — HDR10-compatible single-layer DV (MEL/FEL mapping, profile 4→8.1 conversion)
- **Prometheus metrics** — `/metrics` endpoint on REST API for monitoring (jobs, frames, bytes, uptime)
- **Shell tab completion** — bash, zsh, and fish completion scripts (`imfwizard completions --bash`)

### Desktop GUI (Tauri 2)
- **Dark theme** by default with optional light mode toggle
- **Drag-and-drop** file import (WAV, TTML, image sequences)
- **Timeline editor** — visual segment arrangement for multi-track compositions
- **Asset browser** — browse IMP track files with thumbnails and metadata inspection
- **Keyboard shortcuts** — full shortcut overlay (`?` key), tab navigation (1-9), preview controls
- **Progress bars** — real-time progress tracking for encode/wrap jobs
- **Supplemental IMP wizard** — guided workflow for versioned supplements
- **Loudness metering panel** — EBU R128 / ATSC A/85 compliance badges
- **IMP metadata editor** — edit CPL annotations, content versioning, locale info
- **Delivery preset selector** — one-click configuration for major platforms
- **Preview player** — J2K frame-by-frame playback with waveform display
- **Bitrate analytics dashboard** — per-second charts, histogram, statistics
- **Subtitle burn-in** — GUI for hardcoding subs into video
- **Batch delivery panel** — deliver to multiple platforms with checkboxes
- **IMF-to-DCP converter** — one-click conversion for cinema delivery
- **Job queue manager** — submit, monitor, cancel background jobs
- **Progress notifications** — system notifications when jobs complete
- **Recent projects** — quick access to previously created IMPs

### Packaging & Deployment
- **Docker image** — headless batch processing (`docker run imfwizard create ...`)
- **Flatpak** — Linux desktop distribution via Flathub
- **macOS .dmg** — universal binary with code signing and notarization
- REST API mode with Prometheus-compatible `/metrics` endpoint

### Mastering & Compliance
- **DCDM creation** — Digital Cinema Distribution Master (X'Y'Z' 12/16-bit) as intermediate format
- **Forensic watermarking** — NexGuard, Civolution, or internal spatial watermark embedding
- **Trailer packaging** — ratings cards (MPAA/BBFC/FSK), green/red band, countdown leaders
- **Content version tracker** — SQLite database tracking version history and delivery destinations
- **Accessibility compliance** — verify AD/HI/SL tracks against CVAA, EAA, AODA, Ofcom standards

## Building

```bash
cd rust
cargo build --release
cargo test
```

The Rust workspace uses [postkit](https://github.com/PostPerfection/postkit) and [dcpdoctor-core](https://github.com/PostPerfection/dcpdoctor) as git dependencies. MXF wrapping uses [asdcplib-rs](https://github.com/PostPerfection/asdcplib-rs) FFI bindings.

### Optional runtime dependencies

- **grok** — for image→J2K encoding and decoding (GPU and CPU)
- **ffmpeg** / **ffprobe** — for transcoding, loudness, channel remapping, quality metrics
- **dovi_tool** — for Dolby Vision RPU injection
- **hdr10plus_tool** — for HDR10+ dynamic metadata injection
- **Java** + **Netflix Photon** — for IMF validation
- **AWS CLI** — for S3 upload

### GUI (Tauri 2)

The desktop app uses a single-window layout with sidebar navigation, inspired by professional NLEs.

**GUI features:**
- Drag & drop file import (video, audio, timed text)
- Keyboard shortcuts (Ctrl+N/O/B/P/I, Ctrl+Shift+S for supplement, Ctrl+1–7 for views)
- Recent projects quick-access list
- Right-click context menus on assets (Preview, Remove, Show in Files)
- Asset filter / search
- Auto-detect framerate from imported video (via ffprobe)
- Progress in title bar (visible in taskbar during builds)
- Desktop notifications on build complete/fail
- Conditional button enabling (Build/Preview disabled until ready)
- Built-in mpv preview player (space = play/pause, arrows = seek)

```bash
cd gui
npm install
npm run tauri dev      # development mode
npm run tauri build    # production build
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

### Create an IMP with subtitles and HDR

```bash
imfwizard create \
  --title "My HDR Film" \
  --video /path/to/j2k_frames/ \
  --audio /path/to/audio.wav \
  --subtitle /path/to/subs.ttml \
  --color-space bt2020-pq \
  --output /path/to/output/
```

### Create an IMP from non-J2K images (auto-encode)

```bash
# Input can be DPX, TIFF, EXR, PNG — automatically encoded to J2K
imfwizard create \
  --title "My Film" \
  --video /path/to/dpx_frames/ \
  --audio /path/to/audio.wav \
  --output /path/to/output/
```

### Transcode ProRes to image sequence

```bash
imfwizard transcode \
  -i input.mov \
  -o /path/to/frames/ \
  -f tiff --bit-depth 16
```

### Encode image sequence to JPEG 2000

```bash
imfwizard encode \
  -i /path/to/tiff_frames/ \
  -o /path/to/j2k_output/ \
  --bitrate 250
```

### Create a Supplemental IMP

```bash
imfwizard supplement \
  --title "Version 2" \
  --ov /path/to/original_imp/ \
  --video /path/to/replacement_frames/ \
  --output /path/to/supplemental_imp/ \
  --entry-point 100 --duration 50
```

### Measure loudness

```bash
imfwizard loudness /path/to/audio.wav
```

### Validate an IMP

```bash
imfwizard validate /path/to/imp/
```

### Display IMP info

```bash
imfwizard info /path/to/existing_imp/
```

### Delivery presets

```bash
# Use a preset to auto-configure encoding parameters
imfwizard create \
  --title "Netflix Delivery" \
  --preset netflix \
  --video /path/to/dpx_frames/ \
  --audio /path/to/audio.wav \
  --output /path/to/output/
```

### Batch delivery to multiple platforms

```bash
imfwizard deliver \
  --video /path/to/j2k_frames/ \
  --audio /path/to/audio.wav \
  --title "My Feature" \
  --output /path/to/deliveries/ \
  --targets netflix disney amazon apple
```

### Create IMP from ProRes

```bash
imfwizard prores \
  -i /path/to/master.mov \
  -o /path/to/output_imp/ \
  -t "ProRes Master"
```

### Burn subtitles into video

```bash
imfwizard burn-in \
  -i /path/to/video.mp4 \
  -s /path/to/subs.srt \
  -o /path/to/output_burned.mp4 \
  --font-size 56
```

### Convert IMF to DCP

```bash
imfwizard to-dcp \
  -i /path/to/imp/ \
  -o /path/to/dcp_output/ \
  -t "Cinema Release"
```

### Bitrate analytics

```bash
# Human-readable summary
imfwizard analytics -d /path/to/j2k_frames/

# JSON output for dashboards
imfwizard analytics -d /path/to/j2k_frames/ --json -o report.json
```

### Generate preview thumbnails

```bash
# Single frame
imfwizard preview -d /path/to/j2k/ -o /tmp/thumbs/ -f 42

# Thumbnail strip (10 evenly spaced frames)
imfwizard preview -d /path/to/j2k/ -o /tmp/thumbs/ --strip
```

### REST API server

```bash
# Start on port 9090 with API key auth
imfwizard rest-api --port 9090 --api-key "my-secret" --max-jobs 8

# Endpoints: GET /health, POST /create, POST /validate, POST /encode, POST /transcode, GET /jobs
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
# Compare two IMPs (PSNR + SSIM)
imfwizard compare --imp-a /path/to/imp_v1/ --imp-b /path/to/imp_v2/ --ssim -o report/
```

### Dolby Atmos import

```bash
imfwizard atmos -i atmos_master.bwf -o output_dir/
```

### MCA label generation

```bash
# Generate 5.1 surround MCA labels
imfwizard mca --soundfield 5.1 --language en -o mca_labels.xml

# 7.1 surround
imfwizard mca --soundfield 7.1 --language en -o mca_71.xml
```

### Audio description

```bash
imfwizard audio-desc --main mix_51.wav --description ad_narration.wav -o combined.wav --duck-level -12
```

### Apply 3D LUT

```bash
imfwizard lut --lut grading.cube -i /frames/ -o /graded_frames/
```

### ACES color pipeline

```bash
imfwizard aces -i /log_frames/ -o /aces_frames/ --idt ARRI_LogC4 --odt P3D65_PQ_1000nits
```

### A/V sync check and fix

```bash
# Detect drift
imfwizard av-sync --video /frames/ --audio mix.wav

# Auto-fix
imfwizard av-sync --video /frames/ --audio mix.wav --fix -o fixed.wav
```

### Platform compliance

```bash
# Check Netflix compliance
imfwizard compliance --imp /path/to/imp/ --target netflix

# Check Disney+ compliance
imfwizard compliance --imp /path/to/imp/ --target disney
```

### QC report

```bash
imfwizard qc-report --imp /path/to/imp/ -o report.html --title "Final QC" --client "Studio X"
```

### CPL annotation

```bash
imfwizard annotate --cpl /path/to/CPL.xml --text "Color correction pass 2" --author "Jane" --revision "v1.2"
```

### Partial version (Supplemental IMP)

```bash
imfwizard partial-version --ov /orig_imp/ --video new_reel2.mxf -o /supplement/ \
  --reel 1 --start-frame 1200 --end-frame 2400 --title "Reel 2 fix"
```

### Slate / countdown generation

```bash
# 10-second countdown
imfwizard slate --type countdown -o /pre_roll/ --width 1920 --height 1080

# SMPTE color bars with 1kHz tone
imfwizard slate --type bars -o /bars/ --tone --tone-output reference.wav

# Text slate
imfwizard slate --type slate -o /slate/ --title "MY FILM — Final Master"
```

### Subtitle retiming

```bash
# Retime TTML from 24fps to 25fps
imfwizard retime -i subs_24.ttml -o subs_25.ttml --src-fps-num 24 --tgt-fps-num 25

# Retime SRT from 23.976 to 24fps
imfwizard retime -i subs.srt -o subs_24.srt --src-fps-num 24000 --src-fps-den 1001 --tgt-fps-num 24
```

### SDI output (Blackmagic DeckLink)

```bash
# List available DeckLink devices
imfwizard sdi-preview --list-devices

# Play J2K frames out over SDI on device 0 at 24fps
imfwizard sdi-preview -i /path/to/j2k_frames/ --device 0 --fps-num 24

# Play with embedded audio, loop, UHD mode
imfwizard sdi-preview -i /path/to/j2k/ --audio mix.wav --loop --width 3840 --height 2160

# Play from MXF directly
imfwizard sdi-preview -i video.mxf --device 0
```

## Architecture

```
imfwizard/
├── rust/                # Rust workspace
│   ├── crates/
│   │   ├── imfwizard-core/  # Core IMF creation library
│   │   └── imfwizard-cli/   # CLI binary
│   └── Cargo.toml
├── gui/                 # Tauri 2 desktop application
│   ├── src/             # Frontend (Vite + vanilla JS)
│   └── src-tauri/       # Rust backend (plugin shell)
└── docs/                # GitHub Pages site
```

IMF Wizard shares common functionality with [DCP Wizard](https://github.com/DcpDoctor/dcpwizard)
via the [postkit](https://github.com/DcpDoctor/postkit) library (encoding, transcoding, hashing,
job queue, preferences, REST API, watch folders, and more).

## License

GPL-3.0 — see [LICENSE](LICENSE)
