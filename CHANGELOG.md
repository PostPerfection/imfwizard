# Changelog

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
