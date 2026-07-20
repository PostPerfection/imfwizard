# Changelog

## [Unreleased]

### Added
- **KDM generation** — `kdm` command now generates signed SMPTE 430-1 KDMs via postkit (xmlsec1-verified). New `--signer-cert`/`--signer-key`/`--signer-chain` flags.
- **Subtitle packaging** — `create --subtitle` wraps TTML/IMSC files as AS-02 timed-text MXF and adds a SubtitlesSequence to the CPL.
- **GUI metadata save** — the Metadata panel Save button now calls `metadata-edit` instead of showing a placeholder.

### Changed
- **MXF wrapping** — imfwizard-core delegates J2K/TimedText/Atmos wrapping to postkit's AS-02 writers. PCM now parses the real WAV header (channels/bits/sample rate) instead of hardcoding 5.1/24-bit/48k.
- **target-convert** — maps 2k/4k scope/flat/full to real resolutions and errors on unknown targets instead of always producing 1080p.
- **metadata-edit** — the `--issuer` value is now written (was discarded).
- **Watermark** — visible burn-in only via postkit; removed NexGuard/Civolution options and the `--backend` flag.
- **Atmos import** — wraps essence via asdcplib instead of shelling ffmpeg for the MXF; ADM XML parsed with quick-xml.
- **Timeline/ASSETMAP parsing** — replaced hand-rolled line scanners with quick-xml.

### Removed
- **IMF-to-DCP conversion** — the CLI/GUI paths only ran `ffmpeg -c copy` and never built a DCP; `to-dcp` now fails loud ("IMF to DCP conversion is not implemented").
- **XML-DSIG signing** — `sign_document`/`verify_signature` emitted an empty signature; they now fail loud (postkit's signer is KDM-specific and not exposed for arbitrary IMF XML).

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
