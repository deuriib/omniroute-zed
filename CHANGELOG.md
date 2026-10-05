# Changelog

All notable changes to this project are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-05

### Added

- Host syncer (`omniroute-zed`): one-shot `sync`, looping `watch`
  (default 300 s), and read-only `status` against OmniRoute `/v1/models`.
- JSONC-aware merge into Zed `settings.json`: comments and trailing commas
  survive, curated entries are never touched, missing models are appended,
  nothing is ever removed.
- Atomic writes (temp file + rename), backup on first write, quiet when
  unchanged so Zed does not reload needlessly.
- Companion Zed extension (`extension/`): snippets-only, installable via
  `Install Dev Extension`, no WASM build step.
- CI (`fmt`, `clippy -D warnings`, tests on 3 OSes) and multi-platform
  release workflow (Windows/Linux/macOS binaries attached to tags).
- Dual license: MIT OR Apache-2.0.

[Unreleased]: https://github.com/deuriib/omniroute-zed/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/deuriib/omniroute-zed/releases/tag/v0.1.0
