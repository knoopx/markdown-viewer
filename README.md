# markdown-viewer

A markdown viewer built on [GPUI](https://github.com/zed-industries/gpui) via `gpui-kit`.
Each file opens in its own window, styled by the user's desktop GTK 4 theme (the
built-in default palette as the fallback), and reloads in place — preserving scroll
position — whenever the file changes on disk.

## Usage

```
markdown-viewer <file.md> [file.md ...]
```

- Each path opens in its own centered window (default size 960×720), titled
  `<filename> — markdown-viewer`.
- Every file is read before the GUI launches, so a missing or unreadable path fails
  fast with a clear error instead of an empty window.
- Every open file is watched. Change bursts are debounced (250 ms trailing edge) and
  re-rendered in place, keeping the current scroll position. If a file cannot be
  watched, its window still renders and auto-reload is disabled for that file with a
  message on stderr.
- Page Up / Page Down scroll the viewport by one page; Home / End jump to the top /
  bottom of the document.
- Relative image sources resolve against the markdown file's own directory (e.g.
  `sample.png` next to the file); HTTP(S) URIs load over the network.

## Theme

The appearance is resolved per field by a precedence chain:

1. An explicit `theme.toml` value wins.
2. Otherwise, the user's effective GTK 4 theme, probed at startup via
   `native-theme-gtk` — the desktop palette (background, text, border, accent, link,
   input, and the remaining palette roles), the interface font, and the light/dark
   variant.
3. Otherwise, the built-in defaults — the default palette for the base
   colors (background, foreground, link, border) and the fontconfig system
   sans/monospace families for the fonts.

The GTK 4 probe runs on the main thread before the GPUI event loop claims the
display; when it fails (e.g. headless), the built-in defaults apply.

The default theme's web fonts are not bundled, so the fontconfig system sans-serif
and monospace families stand in for the heading and code fonts respectively.

Rendered content is a centered column with a 780px maximum width and a 1.7 line
height; the column fills the window when the window is narrower than that.

### Configuration

The theme is loaded from `theme.toml` in the project's XDG config directory:
`$XDG_CONFIG_HOME/markdown-viewer/theme.toml`, falling back to the user's default
config directory when `XDG_CONFIG_HOME` is unset.

When the file is absent, the probed GTK 4 theme applies (the built-in defaults when the
probe fails). A partial file overrides only the fields it names; everything left out
keeps the value from the next layer down.

All fields are optional:

| Field | Value | Notes |
| --- | --- | --- |
| `mode` | `"light"` (default) / `"dark"` | Appearance. The probed GTK 4 variant applies only when `theme.toml` is absent. |
| `font_family`, `font_size` | string / pixels | Body font; falls back to the GTK 4 interface font, then the system sans-serif. |
| `mono_font_family`, `mono_font_size` | string / pixels | Code font; falls back to the system monospace. |
| `radius`, `radius_lg` | pixels | Corner radii. |
| `shadow` | bool | Drop shadows. |
| `focus_ring` | bool | Focus rings. |
| `scrollbar_mode` | `"Scrolling"` (default) / `"Hover"` / `"Always"` | Scrollbar visibility. |
| `colors` | table | Named hex colors (`"#rrggbb"` or `"#rrggbbaa"`): `background`, `foreground`, `border`, `accent`, `accent_foreground`, `caret`, `input`, `link`, `group_box`, `button`. |

Example:

```toml
mode = "dark"
font_family = "Inter"
scrollbar_mode = "Hover"

[colors]
background = "#1e1e2e"
foreground = "#cdd6f4"
```

## Building

The project is managed with a Nix flake; the devShell provides the Rust toolchain,
the Vulkan, X11, Wayland, and GTK 4 libraries the GPU backend and the GTK 4 theme
probe need, and sets `RUSTFLAGS` (the Vulkan loader link path) and `LD_LIBRARY_PATH`
(the GUI client libraries wgpu/winit load at runtime), so no manual environment
assembly is required. The flake targets x86_64-linux.

```
nix develop
cargo run -- notes.md
```

## Project layout

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | CLI entry, pre-reading of files, GTK 4 theme probe, theme application, window creation. |
| `src/theme.rs` | Theme precedence (theme.toml > GTK 4 theme > built-in defaults), `theme.toml` loading, the GTK 4 probe, fontconfig system font resolution, the markdown text style. |
| `src/watcher.rs` | `notify`-based file watching with events forwarded over an async channel. |
| `src/window_view.rs` | Per-file window view, debounced reload, scroll-position preservation, keyboard paging. |
