//! Markdown images sized by the image, not by the paragraph around it.
//!
//! gpui-kit sizes a Markdown image one of two ways, and neither fits a
//! screenshot:
//!
//! * An image sharing a paragraph with any text is laid out by the inline
//!   flow, which sizes an image with no explicit `width`/`height` as an inline
//!   glyph: `line_height * 0.75` tall, aspect-scaled. A screenshot followed by
//!   an italic caption on the next source line is ONE CommonMark paragraph, so
//!   the screenshot collapses to a ~20px thumbnail sitting on the caption's
//!   baseline — the "images are minuscule" bug.
//! * An image alone in its paragraph keeps its intrinsic size, but GPUI's
//!   `img` element resolves a definite height from the *intrinsic* width before
//!   any `max_w` cap is applied, so a tall screenshot reserves its full
//!   intrinsic height while `Contain` paints it scaled into the capped width: a
//!   correctly sized picture floating in a box far taller than the picture.
//!
//! `FigureImages` is a [`MarkdownPlugin`] that claims every Markdown image node
//! — and a link whose only child is an image, the badge idiom — and renders it
//! itself: measured at its intrinsic size, scaled *down* (never up) to the
//! content column the window actually draws, with the height recomputed from
//! the aspect ratio so the box hugs the picture. A glyph-sized image (an icon
//! or a shields.io badge) keeps gpui-kit's inline treatment, so it stays on the
//! text line at its natural size.
//!
//! Images whose reference is a `data:` URL are left to gpui-kit: it decodes
//! their bytes inline, which no URI fetch in a resolver could reproduce.

use std::path::{Path, PathBuf};

use gpui_kit::base::text::{
    InlineElement, InlineRenderContext, MarkdownNode, MarkdownParseContext, MarkdownPlugin,
    markdown_ast,
};
use gpui_kit::{
    App, AvailableSpace, ImageSource, IntoElement, ObjectFit, Pixels, SharedUri, Styled,
    StyledImage, Window, img, px,
};

use crate::window_view::MAX_CONTENT_WIDTH;

/// The custom-node name this plugin registers, and the name its renderer is
/// looked up by. Stable: it identifies the nodes this plugin produces.
const FIGURE_NODE: &str = "markdown-viewer-figure";

/// Slack kept off the right edge of the content column.
///
/// The column is centered in the window, but a list item's content box starts
/// one bullet-marker in from it, and a figure inside a list item may not be
/// wider than that narrower box: gpui-kit's inline flow never shrinks a custom
/// object to the wrap width, it lets it overflow. One bullet marker is roughly
/// this wide, so figures fit the narrower box and plain-paragraph figures are
/// only imperceptibly narrower than the text.
const COLUMN_SLACK: f32 = 24.0;

/// The image a figure renders, captured while parsing (which runs off the main
/// thread, where no `Window` or `App` is reachable); the source is resolved and
/// measured later, during rendering.
#[derive(Clone, Debug)]
struct Figure {
    url: String,
}

/// Sizes Markdown images to their intrinsic dimensions, capped at the content
/// column, instead of letting gpui-kit size them as inline glyphs.
pub(crate) struct FigureImages {
    /// The directory relative image sources resolve against — the Markdown
    /// file's own directory. A standalone binary has no asset bundle, so a bare
    /// `assets/screenshot.png` would otherwise be looked up as an embedded
    /// resource and fail silently.
    base_dir: Option<PathBuf>,
}

impl FigureImages {
    pub(crate) fn new(markdown_path: &Path) -> Self {
        Self {
            base_dir: markdown_path.parent().map(PathBuf::from),
        }
    }
}

impl MarkdownPlugin for FigureImages {
    fn name(&self) -> &str {
        FIGURE_NODE
    }

    /// Claims an image node, or a link whose only child is one.
    ///
    /// Claiming the wrapper link matters because that is how badges are
    /// written (`[![build](badge.svg)](https://…)`): gpui-kit only offers its
    /// own image handling once it has descended into the link, so a link is
    /// claimed here to keep the image on this plugin's sizing path. A link with
    /// anything else in it is left to gpui-kit so its text and styling survive.
    fn parse(&self, node: &markdown_ast::Node, _cx: &MarkdownParseContext) -> Option<MarkdownNode> {
        let image = match node {
            markdown_ast::Node::Image(image) => image,
            markdown_ast::Node::Link(link) => match link.children.as_slice() {
                [markdown_ast::Node::Image(image)] => image,
                _ => return None,
            },
            _ => return None,
        };
        if image.url.is_empty() || image.url.starts_with("data:") {
            return None;
        }

        Some(
            MarkdownNode::new(FIGURE_NODE, Figure { url: image.url.clone() })
                // The alt text is what the inline object falls back to (and is
                // read out by) while the picture has no geometry yet, and what
                // a text selection over the figure yields.
                .text(image.alt.clone()),
        )
    }

    /// Renders the figure as an `img` sized from its own measured dimensions.
    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let figure = node.data::<Figure>()?;
        let source = image_source(&figure.url, self.base_dir.as_deref());

        let intrinsic = measure_intrinsic(&source, window, cx);
        if intrinsic.width <= px(0.) || intrinsic.height <= px(0.) {
            // Not loaded (or failed to load) — the element has no geometry to
            // plan against, so return nothing and let the inline object show
            // its text fallback rather than a 0×0 box. The asset load notifies
            // the window, and the next frame measures the real size.
            return None;
        }

        // Scale down to the column, never up: an image smaller than the column
        // keeps its natural size, the way a browser's `max-width: 100%` does.
        let cap = column_width(window);
        let width = intrinsic.width.min(cap);
        let height = intrinsic.height * (f32::from(width) / f32::from(intrinsic.width));

        let picture = img(source)
            .object_fit(ObjectFit::Contain)
            .w(width)
            .h(height);

        Some(if height <= context.line_height() {
            // Glyph-sized (an icon, a status badge): on the text's baseline,
            // exactly where gpui-kit puts an inline image, so it reads as a
            // character in the sentence.
            InlineElement::new(picture).with_baseline(height * 0.75)
        } else {
            // A figure: gpui-kit's default alignment drops its bottom onto the
            // surrounding text's descent, so the line box grows to the picture
            // and nothing else moves.
            InlineElement::new(picture)
        })
    }
}

/// The widest a figure may be: the content column the window draws
/// (`MAX_CONTENT_WIDTH`, or the window itself when it is narrower), less the
/// slack for a list item's marker.
fn column_width(window: &Window) -> Pixels {
    let viewport = f32::from(window.viewport_size().width);
    px((viewport.min(MAX_CONTENT_WIDTH) - COLUMN_SLACK).max(0.))
}

/// The image's own dimensions in logical pixels, or zeros while it has no
/// decoded frame to measure.
///
/// Measured through GPUI's own `img` element so the intrinsic size comes from
/// the same decoded asset the picture is painted from — and so this call also
/// kicks off the load that later frames depend on. `MaxContent` asks for the
/// element's natural size: GPUI gives an unsized `img` its intrinsic pixels.
fn measure_intrinsic(
    source: &ImageSource,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::Size<Pixels> {
    let mut probe = img(source.clone()).into_any_element();
    probe.layout_as_root(
        gpui_kit::size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
        window,
        cx,
    )
}

/// The [`ImageSource`] for a document-provided image reference.
///
/// A scheme-bearing reference is fetched as-is; anything else is a path
/// relative to the Markdown file, which is what makes `![screenshot](assets/…)`
/// next to the file work in a standalone binary with no asset bundle.
pub(crate) fn image_source(uri: &str, base_dir: Option<&Path>) -> ImageSource {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        ImageSource::from(SharedUri::from(uri.to_string()))
    } else if let Some(dir) = base_dir {
        let path = Path::new(uri);
        ImageSource::from(if path.is_absolute() {
            path.to_path_buf()
        } else {
            dir.join(uri)
        })
    } else {
        ImageSource::from(SharedUri::from(uri.to_string()))
    }
}
