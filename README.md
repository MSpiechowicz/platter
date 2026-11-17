# Platter

See what's eating your disk, and clean it up safely. Linux and Windows, fully local, AI-assisted.

> **Status:** early development. Every change on `main` that passes CI is released automatically.

## What it does

- **Analyzes** a disk or folder: how much space is left, what uses it, where, and which file types.
- **Visualizes** the result with animated views (layered "platter" stack, nested map with a 3D column view, gauges, file-age chart).
- **Cleans** quickly and reversibly: caches, build artifacts, old package versions, logs, trash.
- **Advises** with a small AI model that runs in RAM on your machine. It labels each candidate *safe*, *review* or *keep* and explains why. It never deletes anything itself, and a built-in deny-list cannot be overridden by the model.

Nothing leaves your computer. The only network calls are the optional model download and the update check.

## Install

Each command downloads the latest release from GitHub, checks it against the release's `SHA256SUMS` and installs it. All of them are for 64-bit x86.

**Linux, any distribution (AppImage, for your user only, updates itself):**

```sh
curl -fsSL https://github.com/MSpiechowicz/platter/releases/latest/download/install.sh | sh
```

It installs to `~/.local/bin/platter` with a menu entry, and asks for no administrator rights. `sh install.sh --uninstall` removes it again. The graphics card support needs the Vulkan loader (`libvulkan1` or `vulkan-loader`).

**Debian and Ubuntu (deb):**

```sh
curl -fLO https://github.com/MSpiechowicz/platter/releases/latest/download/platter_amd64.deb && sudo apt install ./platter_amd64.deb
```

**Fedora and openSUSE (rpm):**

```sh
sudo dnf install https://github.com/MSpiechowicz/platter/releases/latest/download/platter.x86_64.rpm
```

A deb or rpm install is not updated by `apt` or `dnf` (there is no package repository yet). Platter tells you when a new version is out, and the same command installs it.

**Windows 10 and 11 (PowerShell, for your user only, updates itself):**

```powershell
irm https://github.com/MSpiechowicz/platter/releases/latest/download/install.ps1 | iex
```

It installs to `%LOCALAPPDATA%\Platter` and asks for no administrator rights. The setup is not code-signed yet, so Windows may warn the first time it runs. To remove it: `$env:PLATTER_UNINSTALL = '1'; irm https://github.com/MSpiechowicz/platter/releases/latest/download/install.ps1 | iex`.

## Platforms

Linux and Windows. The OS-specific code sits behind a small platform layer (`platter-platform`), so macOS support can be added without touching the scanner, analysis, cleanup logic or UI.

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
