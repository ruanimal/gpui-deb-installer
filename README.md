# GPUI DEB Installer 📦

[中文](README.zh-CN.md) | **English**

A fast, lightweight graphical installer for `.deb` packages, built with the [GPUI](https://www.gpui.rs/) framework.

![Screenshot](screenshot.png)
![Screenshot2](screenshot2.png)

## Features ✨

- **Fast and fluid** — GPU-accelerated rendering via GPUI keeps the UI instantly responsive.
- **Full package details** — name, version, description, maintainer, installed size, dependencies and more.
- **Installed package management** — browse, search and sort installed packages, with install/update times.
- **File preview** — inspect the contents of a `.deb` before installing it.
- **Desktop integration** — accepts a package path as a command-line argument, so a `.deb` can be opened straight from the file manager's context menu.
- **Safe privileges** — installs and removals run through `pkexec`, keeping every operation under PolicyKit control.

## Requirements 🛠️

- **OS**: Linux (Debian-based distributions such as Ubuntu, Debian, Deepin, UOS)
- **Runtime**: `dpkg`, `apt`, `policykit-1` (`pkexec`)
- **Build**: Rust toolchain, `libvulkan-dev`, `libssl-dev`, `pkg-config`, `libwayland-dev` (when running under Wayland)

## Quick Start 🚀

### 1. Build and run

Make sure Rust is installed first.

```bash
# Run without loading a file
cargo run --release

# Run with a specific deb file
cargo run --release /path/to/your/package.deb
```

### 2. Desktop integration (development)

This workspace ships an `install-icon.sh` script that registers the context-menu entry and icon in your local desktop environment, without installing anything into `/usr/bin`.

```bash
chmod +x install-icon.sh
./install-icon.sh
```

After that you can **right-click a `.deb` file** in your file manager (Nautilus, Thunar, Dolphin, …) and choose **"DEB Installer"**.

### 3. Build a release DEB package

```bash
# Install cargo-deb
cargo install cargo-deb

# Build the .deb package
cargo deb
```

The generated package is written to `target/debian/gpui-deb-installer_*.deb`.
