# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

image-optim is a Rust-based image processing microservice providing REST APIs for image optimization, resizing, cropping, watermarking, and format conversion. It is deployed as a multi-arch Docker image on Docker Hub (`vicanso/image-optim`).

## Commands

```bash
make dev          # Run with bacon hot-reload watcher
make dev-debug    # Run with LOG_LEVEL=5 debug logging
make lint         # Run clippy linter
make lint-fix     # Auto-fix clippy issues
make fmt          # Format code with cargo fmt
make release      # Build optimized release binary
make hooks        # Install pre-commit git hooks (runs fmt + lint before each commit)

cargo test        # Run tests
```

A pre-commit hook (installed via `make hooks`) runs `make fmt && make lint` before each commit.

## Architecture

The service is a single Axum/Tokio HTTP server on port 3000 with 7 modules:

- **`main.rs`** — entry point, Tokio runtime setup, graceful shutdown (SIGTERM/SIGINT with 10s drain)
- **`router.rs`** — Axum router; all image endpoints live under `/images/*`
- **`image.rs`** — HTTP handlers for the four image operations; reads Accept header for auto output-format selection; sets `X-Dssim-Diff` and `X-Ratio` response headers
- **`image_task.rs`** — coordinates calls to the `imageoptimize` crate; caches results (1000 items, 30-min TTL via `cached`)
- **`config.rs`** — TOML config loaded from `configs/` directory; env vars prefixed `IMOP__` override fields (see Configuration below for the separator rules)
- **`dal.rs`** — storage abstraction via OpenDAL (local filesystem by default; S3-compatible via `IMOP__OPENDAL__URL`)
- **`state.rs`** — shared `AppState`; tracks CPU/memory/IO metrics every 60s; enforces `processing_limit` concurrency cap

**API endpoints** (all GET with query params):
- `/images/optim` — compress/optimize, optional format conversion
- `/images/resize` — resize with proportional scaling
- `/images/watermark` — add watermark (Base64-encoded) with positioning
- `/images/crop` — crop a rectangular region
- `/images/command` — returns Markdown API docs

## Configuration

Config files in `configs/` are layered: `default.toml` → `{RUST_ENV}.toml` → env var overrides.

Env var convention: every level boundary uses `__` (double underscore); a single `_` is preserved as part of the field name. `IMOP__OPENDAL__URL` → `opendal.url`; `IMOP__BASIC__MAX_SOURCE_BYTES` → `basic.max_source_bytes` (note the single `_` inside `max_source_bytes` is part of the key name). All `IMOP__`-prefixed vars follow this rule, including the two read directly via `std::env::var` (`IMOP__THREADS`, `IMOP__OPENDAL__<NAME>__URL`).

Key env vars:
| Variable | Default | Description |
|---|---|---|
| `RUST_ENV` | `dev` | Selects `configs/{env}.toml` |
| `IMOP__OPENDAL__URL` | `file://~/Downloads` | Default storage backend URL |
| `IMOP__OPENDAL__<NAME>__URL` | — | Extra named storage; selected by `?source=<name>` |
| `IMOP__OPTIM__QUALITY` | `80` | JPEG/WebP quality 0–100 |
| `IMOP__OPTIM__SPEED` | `3` | AVIF encode speed 1–10 |
| `IMOP__BASIC__MAX_SOURCE_BYTES` | `33554432` | Reject sources whose byte size exceeds this (0 = off; pre-checked via `stat()`) |
| `IMOP__BASIC__MAX_SOURCE_PIXELS` | `100000000` | Reject decoded sources whose `width*height` exceeds this (0 = off) |
| `IMOP__GUARD__DEFAULT_PREFIX_ALLOWLIST` | `[]` | CSV (env) or TOML list of allowed path prefixes for the default storage; empty = unrestricted; named storages inherit this when they have no own entry |
| `IMOP__GUARD__SOURCE_PREFIX_ALLOWLIST__<NAME>` | — | Per-named-storage override; explicit empty string opts that source out of restrictions |
| `IMOP__THREADS` | auto | Tokio worker thread count |

## Linter Rules

`clippy.toml` **denies `unwrap()`** — use `?`, `expect()` with a message, or explicit error handling instead. Cognitive complexity limit is 10 per function. Tests are exempt from the unwrap restriction.

## CI/CD

GitHub Actions (`.github/workflows/build.yml`) triggers on version tags (`v*.*.*`), builds AMD64 and ARM64 release binaries inside Docker, then pushes a multi-arch manifest to Docker Hub.
