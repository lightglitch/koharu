---
title: Development Setup
description: Build the Tauri desktop application, run focused checks, regenerate IPC bindings, and build the docs.
---

# Development Setup

## Prerequisites

- Rust 1.97.1 or later with the Rust 2024 edition toolchain;
- Bun 1.3.14 or later;
- LLVM 22.1.8 or later;
- Ninja 1.13.2 or later;
- platform C/C++ build tools required by native dependencies.
- dav1d 1.3.0 or later, for AVIF page import (see below).

Linux development also needs GTK 3 and the X11 desktop libraries for your distribution. Windows native work uses MSVC build tools.

## AVIF decoding (dav1d)

AVIF page import decodes through dav1d, which the `image` crate links against for its
`avif-native` feature. dav1d is a system library rather than a vendored crate, so it has
to be present before the workspace will build at all. The AVIF *encoder* needs nothing:
it is pure Rust and already enabled.

- Linux: `sudo apt install libdav1d-dev`
- macOS: `brew install dav1d`

Windows has no pkg-config, so install dav1d with vcpkg and point `system-deps` at it
directly:

```bash
vcpkg install dav1d:x64-windows-static-md
```

The `static-md` triplet links dav1d into the binary while keeping the dynamic CRT that
Rust expects, so no DLL ships alongside the app. Then set these once, as user
environment variables:

```
SYSTEM_DEPS_DAV1D_NO_PKG_CONFIG=1
SYSTEM_DEPS_DAV1D_LIB=dav1d
SYSTEM_DEPS_DAV1D_SEARCH_NATIVE=C:/vcpkg/installed/x64-windows-static-md/lib
```

`NO_PKG_CONFIG` is not optional. Without it `dav1d-sys` runs pkg-config regardless of
the other two variables and its build script panics.

## Install and run

```bash
git clone https://github.com/koharu-rs/koharu.git
cd koharu
bun install
bun dev
```

`bun dev` starts the Next.js UI and the Tauri application together. It also builds `koharu-canvas` for WASM once, then watches `koharu-canvas` and `koharu-rasterizer` and regenerates `packages/bridge/src/wasm` after Rust changes. The generated package is part of the Turbopack module graph, so a successful rebuild refreshes the browser client.

## Build

```bash
bun run build
```

The repository build script uses `tauri build --no-bundle`; the executable is written under `target/release`. Installer packaging is performed by the release workflow.

## Focused checks

Choose commands that match the change:

```bash
cargo check -p koharu
cargo test -p koharu-pipeline
cargo fmt --all --check

bun run lint
bun run test
bun run check
bun run --filter @koharu/ui typecheck
```

Do not run end-to-end tests unless the task specifically requires them.

## Generated IPC bindings

Rust command signatures and Specta types are authoritative. Regenerate the TypeScript binding after changing them:

```bash
cargo run -p koharu-app --bin generate
```

Do not hand-edit `packages/bridge/src/protocol.ts`.

## Documentation

Run the single Zensical documentation site locally or build its static output:

```bash
bun run docs:dev
bun run docs:build
```

Content and the single `docs/zensical.toml` configuration live under `docs`. English is rooted at `/`, with Japanese and Simplified Chinese under `/ja-JP/` and `/zh-CN/`; keep all three page sets and the shared navigation structurally identical. Draw diagrams with fenced `mermaid` blocks instead of text or ASCII art.
