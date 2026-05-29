# Changelog

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
- Loudness measurement, subtitle burn-in, IMP-to-DCP conversion, supplemental IMP creation, and target conversion show informational messages (CLI support not yet available)

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
