//! markdown-viewer: renders markdown files with GPUI, one window per file.
//!
//! Each file path given as a CLI argument opens in its own window. The theme is
//! loaded from the XDG config directory when present, underpinned by the
//! built-in defaults. Every open file is
//! watched and reloaded in place (preserving scroll) when it changes on disk.

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use gpui_kit::base::text::TextViewStyle;
use gpui_kit::{
    App, AppContext, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions,
    application, px, size,
};

mod theme;
mod watcher;
mod window_view;

fn main() -> Result<()> {
    let paths: Vec<PathBuf> = env::args().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: markdown-viewer <file.md> [file.md ...]");
        return Ok(());
    }

    // Pre-read every file so a missing or unreadable path fails before the GUI
    // launches.
    let files: Vec<(PathBuf, String)> = paths
        .into_iter()
        .map(|path| {
            std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {path:?}"))
                .map(|content| (path, content))
        })
        .collect::<Result<_>>()?;

    let theme_config = theme::load()?;
    // Probe the user's effective GTK 4 theme on the main thread, before the
    // GPUI event loop claims the display. A failed probe (headless) falls
    // back to the built-in defaults.
    let gtk_theme = theme::probe_gtk();

    application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        // Resolve the system sans and monospace fonts (fontconfig) as the
        // default font families when the theme config names none.
        let system_font = theme::system_font_family().ok();
        let system_mono_font = theme::system_mono_font_family().ok();
        theme::apply(
            cx,
            theme_config.as_ref(),
            gtk_theme.as_ref(),
            system_font.as_deref(),
            system_mono_font.as_deref(),
        );
        let style = theme::text_view_style(
            theme_config.as_ref(),
            gtk_theme.as_ref(),
            system_font.as_deref(),
            system_mono_font.as_deref(),
        );
        for (path, content) in files {
            open_markdown_window(path, content, style.clone(), cx);
        }
    });

    Ok(())
}

/// Opens a single markdown file in its own window.
fn open_markdown_window(path: PathBuf, content: String, style: TextViewStyle, cx: &mut App) {
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("markdown")
        .to_string();

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(960.0), px(720.0)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from(format!("{title} — markdown-viewer"))),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        ..Default::default()
    };

    gpui_kit::open_window(options, cx, move |_, cx| {
        cx.new(|cx| window_view::MarkdownWindow::new(path, content, style, cx))
    })
    // A window-open failure here is a provably rare platform failure (display
    // unavailable); the gpui-kit docs pattern is to expect it away.
    .expect("failed to open markdown window");
}
