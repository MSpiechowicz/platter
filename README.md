# Platter

See what's eating your disk, and clean it up safely. Linux, fully local, AI-assisted.

> **Status:** early development. Nothing is released yet; the sections below describe what is being built.

## What it does

- **Analyzes** a disk or folder: how much space is left, what uses it, where, and which file types.
- **Visualizes** the result with animated views (layered "platter" stack, nested map with a 3D column view, gauges, file-age chart).
- **Cleans** quickly and reversibly: caches, build artifacts, old package versions, logs, trash.
- **Advises** with a small AI model that runs in RAM on your machine. It labels each candidate *safe*, *review* or *keep* and explains why. It never deletes anything itself, and a built-in deny-list cannot be overridden by the model.

Nothing leaves your computer. The only network calls are the optional model download and the update check.

## Platforms

Linux first. The OS-specific code sits behind a small platform layer, so Windows and macOS support can be added without touching the scanner, analysis, cleanup logic or UI.

## Tech

- **Rust** workspace: `platter-core` (domain, scanner, queries), `platter-platform` (OS adapters), `platter-cli` (headless).
- **Tauri 2 + Svelte 5 + TypeScript** desktop app.
- **GitHub Actions** for CI and releases; installs from GitHub Releases with auto-update.

## Development

Prerequisites: rustup, Node 22+, pnpm, and the Tauri 2 Linux system packages (webkit2gtk-4.1 and friends).

```sh
cargo test --workspace          # Rust tests
pnpm install                    # frontend dependencies
pnpm dev                        # frontend in a browser, on the mock API
pnpm -C apps/desktop tauri dev  # full desktop app
```

See [docs/architecture.md](docs/architecture.md) for the design contract.

## License

Platter is free software: you can redistribute it and/or modify it under the terms of the
**GNU General Public License** as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version.

Platter is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without
even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
[LICENSE](LICENSE) file for the full text.
