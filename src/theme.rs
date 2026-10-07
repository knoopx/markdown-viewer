//! Theme configuration loaded from the XDG config directory.
//!
//! The theme file lives at `$XDG_CONFIG_HOME/markdown-viewer/theme.toml`,
//! falling back to `~/.config/markdown-viewer/theme.toml` when
//! `XDG_CONFIG_HOME` is unset. When the file is absent (or a field is left
//! out), the built-in defaults apply; a partial file only
//! overrides the fields it names.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use gpui_kit::base::ScrollbarMode;
use gpui_kit::base::text::TextViewStyle;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    App, Fill, FontWeight, Hsla, Pixels, SharedString, StyleRefinement, px, relative, rems,
};
use serde::{Deserialize, Serialize};

/// Colors resolved from the user's effective GTK 4 theme, via
/// `native_theme_gtk::probe` (the desktop theme after user CSS overrides and
/// the light/dark variant have been applied).
///
/// Every field is optional: the active theme is not required to publish every
/// symbolic color, and an absent field falls through to the next layer of the
/// precedence chain (the built-in defaults for the base colors,
/// "untouched" for the rest).
#[derive(Debug, Clone, Default)]
pub struct GtkTheme {
    /// Whether the resolved window background is dark (relative luminance
    /// below 0.5, the same rule `native-theme-gtk`'s bridge uses); `None`
    /// when the theme exposed no `window_bg_color`.
    pub dark: Option<bool>,
    /// The desktop interface font (`gtk-font-name`), when the desktop sets one.
    pub font_family: Option<String>,
    pub colors: GtkColors,
}

/// The semantic palette roles of `native-theme-gtk` that this app renders
/// with, each mapped onto a gpui-kit theme color field. Every field is
/// optional: the active theme is not required to publish every symbolic
/// color, and an absent field falls through to the next layer of the
/// precedence chain (the built-in defaults for the base colors,
/// "untouched" for the rest).
#[derive(Debug, Clone, Default)]
pub struct GtkColors {
    /// `window_background` (`window_bg_color` / `theme_bg_color`).
    pub background: Option<Hsla>,
    /// `window_text` (`window_fg_color` / `theme_fg_color`).
    pub foreground: Option<Hsla>,
    /// `border` (the `borders` symbolic color; may be translucent — the
    /// active theme's is 15% white, compositing over the page background
    /// into a subtle line).
    pub border: Option<Hsla>,
    /// `accent` (`accent_bg_color` / `accent_color`).
    pub accent: Option<Hsla>,
    /// `accent_text` (`accent_fg_color`).
    pub accent_foreground: Option<Hsla>,
    /// `input_text` (`view_fg_color` / `theme_text_color`).
    pub caret: Option<Hsla>,
    /// `input_background` (`view_bg_color` / `theme_base_color`).
    pub input: Option<Hsla>,
    /// `link` (`link_color`, falling back to the accent in the probe).
    pub link: Option<Hsla>,
    /// `surface_background` (raised surfaces: `card_bg_color` / popover).
    pub group_box: Option<Hsla>,
    /// `surface_text` (text on raised surfaces) — the GroupBox foreground.
    pub surface_text: Option<Hsla>,
    /// `selection_background` — the text-selection color.
    pub selection_background: Option<Hsla>,
    /// `error` — the destructive (danger) color.
    pub error: Option<Hsla>,
    /// `warning`.
    pub warning: Option<Hsla>,
    /// `success`.
    pub success: Option<Hsla>,
}

/// The built-in default color palette, as 24-bit hex values.
mod defaults {
    /// Page background (`--c-bg`).
    pub const BACKGROUND: u32 = 0xFF_FC_F9;
    /// Body text (`--c-text`).
    pub const FOREGROUND: u32 = 0x13_20_2C;
    /// Links (`--c-link`).
    pub const LINK: u32 = 0x3E_32_82;
    /// Code-block border (`--border-ui`).
    pub const BORDER: u32 = 0xE7_EA_ED;
    /// Table row separator (`--border-table`).
    pub const TABLE_BORDER: u32 = 0xCC_CC_CC;
}

/// Builds a fully-opaque [`Hsla`] from a 24-bit `0xRRGGBB` hex value.
///
/// `gpui_kit::rgba` reads a `0xRRGGBBAA` word (alpha in the low byte), so the
/// opaque alpha byte is appended to the shifted color.
fn hex(color: u32) -> Hsla {
    Hsla::from(gpui_kit::rgba((color << 8) | 0xFF))
}

/// The theme configuration accepted from `theme.toml`.
///
/// Every field is optional (or defaults) so a partial file only overrides what
/// it names; anything left out keeps the built-in default.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeConfig {
    /// Light or dark appearance (`"light"` / `"dark"`); defaults to light.
    pub mode: ThemeMode,
    pub font_family: Option<String>,
    pub font_size: Option<Pixels>,
    pub mono_font_family: Option<String>,
    pub mono_font_size: Option<Pixels>,
    pub radius: Option<Pixels>,
    pub radius_lg: Option<Pixels>,
    pub shadow: Option<bool>,
    pub focus_ring: Option<bool>,
    /// `"Scrolling"` (default), `"Hover"`, or `"Always"`.
    pub scrollbar_mode: Option<ScrollbarMode>,
    pub colors: Option<ThemeColors>,
}

/// A subset of the theme color palette. Each color is a hex string
/// (`"#rrggbb"` or `"#rrggbbaa"`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeColors {
    pub background: Option<Hsla>,
    pub foreground: Option<Hsla>,
    pub border: Option<Hsla>,
    pub accent: Option<Hsla>,
    pub accent_foreground: Option<Hsla>,
    pub caret: Option<Hsla>,
    pub input: Option<Hsla>,
    pub link: Option<Hsla>,
    pub group_box: Option<Hsla>,
    pub button: Option<Hsla>,
}

/// Resolves the theme config path: `$XDG_CONFIG_HOME/markdown-viewer/theme.toml`,
/// or `~/.config/markdown-viewer/theme.toml` when `XDG_CONFIG_HOME` is unset.
fn theme_config_path() -> Result<PathBuf> {
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home =
                std::env::var("HOME").context("XDG_CONFIG_HOME is unset and HOME is not set")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join("markdown-viewer").join("theme.toml"))
}

/// Loads the theme config, returning `Ok(None)` when the file is absent.
///
/// Parse errors propagate with context.
pub fn load() -> Result<Option<ThemeConfig>> {
    let path = theme_config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read theme config {path:?}"))?;
    let config: ThemeConfig = toml::from_str(&contents)
        .with_context(|| format!("failed to parse theme config {path:?}"))?;
    Ok(Some(config))
}

/// Resolves the system-configured sans-serif font family via fontconfig
/// (`fc-match`), as the default font family when the theme config names none.
pub fn system_font_family() -> Result<String> {
    let output = Command::new("fc-match")
        .args(["-f", "%{family[0]}\n", "sans"])
        .output()
        .context("failed to run fc-match to resolve the system font")?;
    if !output.status.success() {
        bail!(
            "fc-match exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let family = String::from_utf8(output.stdout)
        .context("fc-match output is not valid UTF-8")?
        .trim()
        .to_string();
    if family.is_empty() {
        bail!("fc-match returned an empty font family");
    }
    Ok(family)
}

/// Resolves the system-configured monospace font family via fontconfig
/// (`fc-match`), as the default code font family when the theme config names
/// none.
pub fn system_mono_font_family() -> Result<String> {
    let output = Command::new("fc-match")
        .args(["-f", "%{family[0]}\n", "monospace"])
        .output()
        .context("failed to run fc-match to resolve the system monospace font")?;
    if !output.status.success() {
        bail!(
            "fc-match exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let family = String::from_utf8(output.stdout)
        .context("fc-match output is not valid UTF-8")?
        .trim()
        .to_string();
    if family.is_empty() {
        bail!("fc-match returned an empty font family");
    }
    Ok(family)
}

/// Converts a `native_theme_gtk::Rgba` (f32 0..=1 channels) to an
/// [`Hsla`], preserving the alpha channel — theme symbolic colors may be
/// translucent (the active theme's `borders` is 15% white, meant to
/// composite over the page background into a subtle line) — using the same
/// `0xRRGGBBAA` word shape as [`hex`].
fn gtk_rgba(color: &native_theme_gtk::Rgba) -> Hsla {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    let word = (to_u8(color.red) << 24)
        | (to_u8(color.green) << 16)
        | (to_u8(color.blue) << 8)
        | to_u8(color.alpha);
    Hsla::from(gpui_kit::rgba(word))
}

/// The WCAG relative luminance of a color, the dark-mode test
/// `native-theme-gtk` applies to the window background.
fn relative_luminance(color: &native_theme_gtk::Rgba) -> f32 {
    fn linear(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
}

/// Probes the user's effective GTK 4 theme.
///
/// Runs `native_theme_gtk::probe` on the calling (main) thread **before** the
/// GPUI event loop claims the display: the probe initializes GTK's global
/// state, and no GTK event loop is started, so GPUI can run afterward in the
/// same process. Returns `None` when GTK cannot be initialized (no display,
/// headless) so the caller falls back to the built-in defaults.
pub fn probe_gtk() -> Option<GtkTheme> {
    let snapshot = native_theme_gtk::probe().ok()?;
    let palette = &snapshot.semantic_palette;
    Some(GtkTheme {
        dark: palette
            .window_background
            .as_ref()
            .map(|c| relative_luminance(c) < 0.5),
        font_family: snapshot.settings.font_name.clone(),
        colors: GtkColors {
            background: palette.window_background.as_ref().map(gtk_rgba),
            foreground: palette.window_text.as_ref().map(gtk_rgba),
            border: palette.border.as_ref().map(gtk_rgba),
            accent: palette.accent.as_ref().map(gtk_rgba),
            accent_foreground: palette.accent_text.as_ref().map(gtk_rgba),
            caret: palette.input_text.as_ref().map(gtk_rgba),
            input: palette.input_background.as_ref().map(gtk_rgba),
            link: palette.link.as_ref().map(gtk_rgba),
            group_box: palette.surface_background.as_ref().map(gtk_rgba),
            surface_text: palette.surface_text.as_ref().map(gtk_rgba),
            selection_background: palette.selection_background.as_ref().map(gtk_rgba),
            error: palette.error.as_ref().map(gtk_rgba),
            warning: palette.warning.as_ref().map(gtk_rgba),
            success: palette.success.as_ref().map(gtk_rgba),
        },
    })
}

/// Applies `cfg` to the global theme, with the GTK 4 theme and the built-in
/// defaults underpinning it.
///
/// Precedence, per field: an explicit `theme.toml` value wins; otherwise the
/// probed GTK 4 theme value is used; otherwise the built-in default (the fontconfig
/// system font for families). Appearance and fonts are applied in one
/// `Theme::update`; colors are applied in a second `Theme::update` because
/// setting `theme.mode` in the first update loads that mode's registered
/// theme, which would overwrite any colors set in the same closure.
pub fn apply(
    cx: &mut App,
    cfg: Option<&ThemeConfig>,
    gtk: Option<&GtkTheme>,
    system_font: Option<&str>,
    system_mono_font: Option<&str>,
) {
    let body_family = cfg
        .and_then(|c| c.font_family.clone())
        .or_else(|| gtk.and_then(|g| g.font_family.clone()))
        .or_else(|| system_font.map(|f| f.to_string()));
    let mono_family = cfg
        .and_then(|c| c.mono_font_family.clone())
        .or_else(|| system_mono_font.map(|f| f.to_string()));

    Theme::update(cx, |theme| {
        if let Some(c) = cfg {
            theme.mode = c.mode;
        } else if let Some(dark) = gtk.and_then(|g| g.dark) {
            // The theme config names no mode: follow the probed GTK variant.
            theme.mode = if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
        }
        if let Some(family) = &body_family {
            theme.font_family = SharedString::from(family.as_str());
        }
        // the built-in base font size is 18px.
        theme.font_size = cfg.and_then(|c| c.font_size).unwrap_or(px(18.));
        if let Some(family) = &mono_family {
            theme.mono_font_family = SharedString::from(family.as_str());
        }
        if let Some(size) = cfg.and_then(|c| c.mono_font_size) {
            theme.mono_font_size = size;
        }
        if let Some(radius) = cfg.and_then(|c| c.radius) {
            theme.radius = radius;
        }
        if let Some(radius) = cfg.and_then(|c| c.radius_lg) {
            theme.radius_lg = radius;
        }
        if let Some(shadow) = cfg.and_then(|c| c.shadow) {
            theme.shadow = shadow;
        }
        if let Some(ring) = cfg.and_then(|c| c.focus_ring) {
            theme.focus_ring = ring;
        }
        if let Some(mode) = cfg.and_then(|c| c.scrollbar_mode) {
            theme.scrollbar_mode = mode;
        }
    });

    // Colors: built-in defaults underpin, the GTK 4 theme fills what the base
    // palette does not define, and config values win. Only the base colors are
    // defaulted; the rest keep the existing "None = untouched" behavior.
    let colors = cfg.and_then(|c| c.colors.as_ref());
    let gtk_colors = gtk.map(|g| &g.colors);
    Theme::update(cx, |theme| {
        theme.colors.background = colors
            .and_then(|c| c.background)
            .or(gtk_colors.and_then(|c| c.background))
            .unwrap_or(hex(defaults::BACKGROUND));
        theme.colors.foreground = colors
            .and_then(|c| c.foreground)
            .or(gtk_colors.and_then(|c| c.foreground))
            .unwrap_or(hex(defaults::FOREGROUND));
        theme.colors.link = colors
            .and_then(|c| c.link)
            .or(gtk_colors.and_then(|c| c.link))
            .unwrap_or(hex(defaults::LINK));
        theme.colors.border = colors
            .and_then(|c| c.border)
            .or(gtk_colors.and_then(|c| c.border))
            .unwrap_or(hex(defaults::BORDER));
        if let Some(c) = colors {
            if let Some(v) = c.accent {
                theme.colors.accent = v;
            }
            if let Some(v) = c.accent_foreground {
                theme.colors.accent_foreground = v;
            }
            if let Some(v) = c.caret {
                theme.colors.caret = v;
            }
            if let Some(v) = c.input {
                theme.colors.input = v;
            }
            if let Some(v) = c.group_box {
                theme.colors.group_box = v;
            }
            if let Some(v) = c.button {
                theme.colors.button = v;
            }
        } else if let Some(g) = gtk_colors {
            // No theme.toml colors: the GTK 4 theme fills the non-base fields.
            if let Some(v) = g.accent {
                theme.colors.accent = v;
            }
            if let Some(v) = g.accent_foreground {
                theme.colors.accent_foreground = v;
            }
            if let Some(v) = g.caret {
                theme.colors.caret = v;
            }
            if let Some(v) = g.input {
                theme.colors.input = v;
            }
            if let Some(v) = g.group_box {
                theme.colors.group_box = v;
            }
            if let Some(v) = g.surface_text {
                theme.colors.group_box_foreground = v;
            }
            if let Some(v) = g.selection_background {
                theme.colors.selection = v;
            }
            if let Some(v) = g.error {
                theme.colors.danger = v;
            }
            if let Some(v) = g.warning {
                theme.colors.warning = v;
            }
            if let Some(v) = g.success {
                theme.colors.success = v;
            }
        }
        // Table rows: gpui-component's Table paints row backgrounds from its
        // own ThemeColor fields; the registered dark theme's near-black
        // `table` value renders the rows pure black on the dark page, so
        // every table color is drawn from the resolved theme colors set
        // above (theme.toml > GTK 4 theme > built-in fallback).
        theme.colors.table = theme.colors.background;
        theme.colors.table_even = theme.colors.background;
        theme.colors.table_head = theme.colors.background;
        theme.colors.table_foot = theme.colors.background;
        theme.colors.table_head_foreground = theme.colors.foreground;
        theme.colors.table_foot_foreground = theme.colors.foreground;
        theme.colors.table_hover = gtk_colors
            .and_then(|c| c.input)
            .unwrap_or(theme.colors.background);
        theme.colors.table_row_border = theme.colors.border;
        // Markdown tables render through gpui-base's text path, whose data-row
        // background falls back to the base theme's `surface` token, which
        // derives from this `popover` color (gpui-component `color_tokens()`);
        // the registered dark theme's near-black popover renders the rows
        // near-black on the dark page, so it is drawn from the resolved page
        // background set above.
        theme.colors.popover = theme.colors.background;
    });
}

/// Builds the base [`TextViewStyle`] that renders markdown, with every color
/// drawn from the resolved theme (theme.toml > GTK 4 theme > built-in
/// fallback).
pub fn text_view_style(
    cfg: Option<&ThemeConfig>,
    gtk: Option<&GtkTheme>,
    system_font: Option<&str>,
    system_mono_font: Option<&str>,
) -> TextViewStyle {
    let colors = cfg.and_then(|c| c.colors.as_ref());
    let gtk_colors = gtk.map(|g| &g.colors);
    let foreground = colors
        .and_then(|c| c.foreground)
        .or(gtk_colors.and_then(|c| c.foreground))
        .unwrap_or(hex(defaults::FOREGROUND));
    let link = colors
        .and_then(|c| c.link)
        .or(gtk_colors.and_then(|c| c.link))
        .unwrap_or(hex(defaults::LINK));
    let border = colors
        .and_then(|c| c.border)
        .or(gtk_colors.and_then(|c| c.border))
        .unwrap_or(hex(defaults::TABLE_BORDER));
    let background = colors
        .and_then(|c| c.background)
        .or(gtk_colors.and_then(|c| c.background))
        .unwrap_or(hex(defaults::BACKGROUND));
    let accent = colors
        .and_then(|c| c.accent)
        .or(gtk_colors.and_then(|c| c.accent));
    let code_background = colors
        .and_then(|c| c.input)
        .or(gtk_colors.and_then(|c| c.input))
        .unwrap_or(background);
    let inline_code = accent.unwrap_or(foreground);

    // The default theme's heading font is an unbundled woff2 asset, so the
    // fontconfig system sans font stands in when resolvable.
    let heading_family: Option<SharedString> = system_font.map(SharedString::from);
    // The heading closure moves its own copy; the table-head refinement uses
    // the original.
    let heading_family_for_closure = heading_family.clone();
    // The default theme's code font is an unbundled woff2 asset, so the
    // fontconfig system monospace font stands in when resolvable.
    let mono_family: Option<SharedString> = system_mono_font.map(SharedString::from);

    TextViewStyle::default()
        .with_foreground(foreground)
        // Blockquote text uses the muted foreground; the default palette colors it like the
        // body text, so it is the same value.
        .with_muted_foreground(foreground)
        .with_link(link)
        .with_code_background(code_background)
        // Shared border (API constraint): the blockquote left rule, hr, and
        // table row separators all share this one color; the reference theme's
        // blockquote and hr colors cannot be expressed
        // separately, so the table row-separator color #ccc is carried.
        .with_border(border)
        // block spacing: p/blockquote/ul/ol/dl/table margin 0.8em 0.
        .with_paragraph_gap(rems(0.8))
        // The per-level color split is dropped: every heading level renders in
        // the resolved foreground, keeping the per-level sizes and weights.
        .with_heading(move |level| {
            heading_refinement(level, heading_family_for_closure.as_ref(), foreground)
        })
        // The code-block border uses the resolved shared border color (dark-
        // mode aware), not a hardcoded light default.
        .with_code_block(code_block_refinement(border, mono_family.as_ref()))
        .with_table(table_refinement())
        .with_table_head(table_head_refinement(background, heading_family.as_ref()))
        .with_table_cell(table_cell_refinement())
        // Inline code keeps the API's 1rem font size: `HighlightStyle` has no
        // font-size field, so 1rem is the 18px body size the API already uses.
        .with_inline_code(gpui_kit::HighlightStyle {
            color: Some(inline_code),
            ..Default::default()
        })
}

/// The per-level heading refinement: the default sizes, weights, margins, and color.
fn heading_refinement(level: u8, family: Option<&SharedString>, color: Hsla) -> StyleRefinement {
    let (size, weight) = match level {
        1 => (rems(1.9), FontWeight::BOLD),
        2 => (rems(1.6), FontWeight::BOLD),
        3 => (rems(1.3), FontWeight::SEMIBOLD),
        4 => (rems(1.15), FontWeight::SEMIBOLD),
        // h5 and h6 share the 1rem size.
        _ => (rems(1.), FontWeight::SEMIBOLD),
    };
    let mut r = StyleRefinement::default();
    r.text.color = Some(color);
    r.text.font_weight = Some(weight);
    r.text.font_size = Some(size.into());
    if let Some(family) = family {
        r.text.font_family = Some(family.clone());
    }
    r.text.line_height = Some(relative(1.3));
    // In the virtualized list (gpui-base `render_root`), a list item's
    // margins produce no visible spacing — only padding does — and the style
    // API has no sibling context, so per-level top margins are not
    // expressible: one 1rem top padding serves every heading.
    r.padding.top = Some(rems(1.).into());
    // 1rem bottom margin (overrides the library's base
    // 0.3rem heading bottom padding).
    r.padding.bottom = Some(rems(1.).into());
    r
}

/// The code-block refinement (padding, border, radius, size, line height,
/// code font); the background comes from `with_code_background`, and `border`
/// is the resolved shared border color.
fn code_block_refinement(border: Hsla, mono_family: Option<&SharedString>) -> StyleRefinement {
    let mut r = StyleRefinement::default();
    // Left/right padding keeps the base 12px.
    r.padding.top = Some(px(8.).into());
    r.padding.bottom = Some(px(6.).into());
    set_all_edges(&mut r.border_widths, px(1.).into());
    r.border_color = Some(border);
    set_all_corners(&mut r.corner_radii, px(3.).into());
    r.text.font_size = Some(rems(0.8).into());
    r.text.line_height = Some(relative(1.4));
    // The code font falls back to monospace: the woff2 asset is not bundled,
    // so the resolved system monospace family stands in.
    if let Some(family) = mono_family {
        r.text.font_family = Some(family.clone());
    }
    r
}

/// The table refinement: no outer border (overrides the base 1px) and the
/// default 0.8rem table font size.
fn table_refinement() -> StyleRefinement {
    let mut r = StyleRefinement::default();
    set_all_edges(&mut r.border_widths, px(0.).into());
    r.text.font_size = Some(rems(0.8).into());
    r
}

/// The table-header refinement. The page-background fill mimics "no header
/// background": the default theme specifies no `th` background, but the API
/// forces the header row onto the shared code-background. Cell padding comes
/// from `table_cell_refinement` (the base header row div carries none).
fn table_head_refinement(background: Hsla, family: Option<&SharedString>) -> StyleRefinement {
    let mut r = StyleRefinement {
        background: Some(Fill::from(background)),
        ..Default::default()
    };
    if let Some(family) = family {
        r.text.font_family = Some(family.clone());
    }
    r.text.font_weight = Some(FontWeight::BOLD);
    r
}

/// The table-cell refinement: 6px vertical cell padding and zeroed border
/// widths, which remove the library's base vertical column borders (tables
/// carry row separators only).
fn table_cell_refinement() -> StyleRefinement {
    let mut r = StyleRefinement::default();
    r.padding.top = Some(px(6.).into());
    r.padding.bottom = Some(px(6.).into());
    r.padding.left = Some(px(0.).into());
    r.padding.right = Some(px(0.).into());
    set_all_edges(&mut r.border_widths, px(0.).into());
    r
}

/// Sets all four edges of an [`EdgesRefinement`] to `value`.
fn set_all_edges<E>(edges: &mut gpui_kit::EdgesRefinement<E>, value: E)
where
    E: Clone + std::fmt::Debug + Default + PartialEq,
{
    edges.top = Some(value.clone());
    edges.bottom = Some(value.clone());
    edges.left = Some(value.clone());
    edges.right = Some(value.clone());
}

/// Sets all four corners of a [`CornersRefinement`] to `value`.
fn set_all_corners<E>(corners: &mut gpui_kit::CornersRefinement<E>, value: E)
where
    E: Clone + std::fmt::Debug + Default + PartialEq,
{
    corners.top_left = Some(value.clone());
    corners.top_right = Some(value.clone());
    corners.bottom_left = Some(value.clone());
    corners.bottom_right = Some(value.clone());
}
