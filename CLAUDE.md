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
- **`config.rs`** — TOML config loaded from `configs/` directory; env vars prefixed `IMOP_` override fields
- **`dal.rs`** — storage abstraction via OpenDAL (local filesystem by default; S3-compatible via `IMOP_OPENDAL_URL`)
- **`state.rs`** — shared `AppState`; tracks CPU/memory/IO metrics every 60s; enforces `processing_limit` concurrency cap

**API endpoints** (all GET with query params):
- `/images/optim` — compress/optimize, optional format conversion
- `/images/resize` — resize with proportional scaling
- `/images/watermark` — add watermark (Base64-encoded) with positioning
- `/images/crop` — crop a rectangular region
- `/images/command` — returns Markdown API docs

## Configuration

Config files in `configs/` are layered: `default.toml` → `{RUST_ENV}.toml` → env var overrides.

Key env vars:
| Variable | Default | Description |
|---|---|---|
| `RUST_ENV` | `dev` | Selects `configs/{env}.toml` |
| `IMOP_OPENDAL_URL` | `file://~/Downloads` | Storage backend URL |
| `IMOP_OPTIM_QUALITY` | `80` | JPEG/WebP quality 0–100 |
| `IMOP_OPTIM_SPEED` | `3` | AVIF encode speed 1–10 |
| `IMAGE_OPTIM_THREADS` | auto | Tokio worker thread count |

## Linter Rules

`clippy.toml` **denies `unwrap()`** — use `?`, `expect()` with a message, or explicit error handling instead. Cognitive complexity limit is 10 per function. Tests are exempt from the unwrap restriction.

## CI/CD

GitHub Actions (`.github/workflows/build.yml`) triggers on version tags (`v*.*.*`), builds AMD64 and ARM64 release binaries inside Docker, then pushes a multi-arch manifest to Docker Hub.
