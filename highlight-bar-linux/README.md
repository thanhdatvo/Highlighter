# Highlight Bar — Linux/X11 Rust port

This is a **functional X11 prototype**, not a complete one-to-one AppKit port. It implements the reading bar, spotlight, tint, presets, custom hex colors, geometry, JSON settings, custom profiles, locking, and global shortcuts using X11.

## Requirements

- A Linux **X11/Xorg session** (`echo $XDG_SESSION_TYPE` should print `x11`). An Xwayland display under Wayland is **not equivalent**: it cannot guarantee overlay placement or global shortcuts across native Wayland windows.
- An X11 compositing window manager (such as picom, KWin/X11, Xfwm with compositor, or Mutter/Xorg) to blend the ARGB transparency.
- Rust / Cargo (`rustup` or the distribution's `cargo` package).

## Build and run

```bash
sudo apt install build-essential pkg-config libx11-dev libxcb1-dev  # Debian/Ubuntu
cargo build --release
cargo run --release
```

All rendering and X11 calls are performed by `x11rb` without GTK or AppKit. The app is controlled through its terminal stdin; type `help` for commands. Settings are saved at `~/.config/highlight-bar-linux/settings.json` (or `$XDG_CONFIG_HOME/highlight-bar-linux/settings.json`). Profiles are in `profiles.json`.

Examples:

```text
mode spotlight
size 26
color #FFCC00
spotlight-color Gray
spotlight-opacity 0.6
orientation vertical
shape line
save-profile Reading
profile ADHD / Focus
mode tint
tint-color #FBE6AA
tint-opacity 0.2
```

Global shortcuts **Super+Shift+H**, **Super+Shift+L**, **Super+Shift+Up/Down** (nudging only while locked) replace the macOS Command shortcuts. Shortcut registration may fail if another program owns a combination. The application runs as long as the terminal remains open; `quit` shuts it down.

## Differences from the macOS version

- No GTK menu/tray, graphical color dialog, onboarding window, slider, or VoiceOver/accessibility swatches. Use the terminal commands or edit the settings JSON instead.
- The bar and spotlight currently have **square corners** rather than rounded corners.
- **Mouse** tracking works. `tracking scroll` is not a full system-wide scroll monitor on X11 as currently implemented; `tracking keyboard` responds only to the registered Super+Shift+Up/Down keys. Choose `tracking mouse` for normal operation.
- Per-app auto-enable fields are stored for JSON compatibility, but per-app activation filtering is **not implemented**.
- Login item, Sparkle updates, multi-monitor hot-plug window behavior beyond basic XRandR detection, and a native desktop tray are not full ports.
- Window/input transparency requires X11 Shape and ARGB visual/compositing support. The program intentionally fails rather than pretending to provide Wayland support.
- Performance: spotlight updates upload a full root-window RGBA pixel buffer when the pointer moves. This is simple but may use significant CPU/RAM on 4K or multi-display desktops. A production port should use XRender/GL and a dedicated event source.

## Test

```bash
cargo test
```

**Build status:** This project was written and statically reviewed in an environment without Rust/Cargo, so it has **not been compiled or run on a Linux desktop**. Minor API/build adjustments may be needed with your installed `x11rb` version.
