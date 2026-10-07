//! A markdown window: renders a file's markdown and auto-reloads it on change,
//! preserving the scroll position across the asynchronous re-parse.
//!
//! The scroll is owned by an OUTER scrollable div (not the TextView). The
//! `TextView` is non-scrollable — it expands to fit all of its content — so
//! the outer div's scroll handle is what moves the content. The vertical
//! padding (30px top / 100px bottom) and the centered `MAX_CONTENT_WIDTH`
//! column are applied to a div INSIDE the scroll, so the padding scrolls with
//! the content instead of insetting the scroll viewport.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::base::{
    Scrollbar, ScrollbarMode,
    text::{TextView, TextViewState, TextViewStyle},
};
use gpui_kit::{
    AppContext, AsyncApp, Context, Entity, FocusHandle, ImageSource, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Render, ScrollHandle, SharedUri,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, point, px, relative,
};
use smol::Timer;

use crate::watcher::FileWatcher;

/// How long to wait for further file events before treating a burst as settled.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Line-height ratio of the rendered text.
const LINE_HEIGHT: f32 = 1.7;
/// Maximum content column width, in pixels. The column is centered inside the
/// actual window width (no hardcoded window size): it fills the window when
/// the window is narrower than this, and is centered with equal side margins
/// when it is wider.
const MAX_CONTENT_WIDTH: f32 = 780.0;
/// Top content padding, in pixels.
const TOP_PADDING: f32 = 30.0;
/// Bottom content padding, in pixels.
const BOTTOM_PADDING: f32 = 100.0;

/// Views a single markdown file in its own window and keeps it in sync with the
/// file on disk.
pub struct MarkdownWindow {
    view_state: Entity<TextViewState>,
    /// The text-view style applied to the markdown content.
    text_style: TextViewStyle,
    /// Focus target for the window; owns the Page Up/Down key handler.
    focus_handle: FocusHandle,
    /// True until the first render has moved focus to `focus_handle`.
    needs_focus: bool,
    /// The file being viewed; its parent directory is the base for resolving
    /// relative image sources (e.g. `sample.png` next to the markdown file).
    path: PathBuf,
    /// The outer scrollable div's scroll handle. The TextView is non-scrollable
    /// (it expands to fit its content), so this handle — bound to the outer
    /// div via `track_scroll` — is what owns the scroll offset.
    scroll_handle: ScrollHandle,
    /// The scroll offset (y) to restore after a reload, until the new
    /// document's size is stable for two consecutive frames. `None` when not
    /// restoring. The offset is negative (scrolled down) and lives in
    /// `[-max_offset, 0]`.
    scroll_restore_offset: Option<Pixels>,
    /// The outer div's content height (its max scroll offset) last observed
    /// while restoring; used to detect the frame the new document's size
    /// settles, frame to frame.
    restore_content_height: Option<Pixels>,
    /// Consecutive frames with an unchanged content height while restoring.
    restore_stable_frames: u32,
    /// Kept so the file watcher stays alive for the lifetime of the window.
    _watcher: Option<FileWatcher>,
}

impl MarkdownWindow {
    /// Creates the window view for `path` with initial `text`, starts the file
    /// watcher, and spawns the debounced reload task.
    pub fn new(
        path: PathBuf,
        text: String,
        text_style: TextViewStyle,
        cx: &mut Context<Self>,
    ) -> Self {
        let view_state = cx.new(|cx| TextViewState::markdown(&text, cx));
        let focus_handle = cx.focus_handle();
        // The outer div's scroll handle; created up front so the render can
        // bind it via `track_scroll`.
        let scroll_handle = ScrollHandle::new();

        let mut watcher = None;
        if let Ok((file_watcher, receiver)) = FileWatcher::new(&path) {
            let watched = path.clone();
            cx.spawn(
                async move |this: WeakEntity<MarkdownWindow>, cx: &mut AsyncApp| {
                    while receiver.recv().await.is_ok() {
                        // Trailing-edge debounce: after the last event, wait for
                        // DEBOUNCE of quiet; a further event extends the window.
                        loop {
                            Timer::after(DEBOUNCE).await;
                            if receiver.try_recv().is_err() {
                                break; // Quiet for DEBOUNCE — the burst is settled.
                            }
                        }

                        match std::fs::read_to_string(&watched) {
                            Ok(new_text) => {
                                if let Err(err) =
                                    this.update(cx, |view, cx| view.reload(&new_text, cx))
                                {
                                    eprintln!(
                                        "markdown-viewer: reload failed for {watched:?}: {err}"
                                    );
                                }
                            }
                            Err(err) => {
                                eprintln!("markdown-viewer: failed to re-read {watched:?}: {err}");
                            }
                        }
                    }
                },
            )
            .detach();
            watcher = Some(file_watcher);
        } else {
            eprintln!("markdown-viewer: could not watch {path:?}; auto-reload disabled");
        }

        Self {
            view_state,
            text_style,
            path,
            focus_handle,
            needs_focus: true,
            scroll_handle,
            scroll_restore_offset: None,
            restore_content_height: None,
            restore_stable_frames: 0,
            _watcher: watcher,
        }
    }

    /// Scroll the rendered content by one viewport height (a "page").
    /// `down` true scrolls toward the bottom, false toward the top.
    ///
    /// The scroll offset lives in `[-max_offset, 0]` (negative = scrolled
    /// down), so paging down subtracts and paging up adds the page height,
    /// clamped to that range.
    fn scroll_page(&mut self, down: bool, cx: &mut Context<Self>) {
        let handle = &self.scroll_handle;
        // The page height is the scroll viewport's height (the outer div's
        // bounds). Before the first layout pass the bounds are zero, in which
        // case there is nothing to page by.
        let page = f32::from(handle.bounds().size.height);
        if page <= 0.0 {
            return;
        }
        let current = f32::from(handle.offset().y);
        let max_offset = f32::from(handle.max_offset().y);
        let delta = if down { -page } else { page };
        let new_offset = (current + delta).clamp(-max_offset, 0.0);
        handle.set_offset(point(px(0.), px(new_offset)));
        // `set_offset` updates the scroll state but does not mark the view
        // dirty (unlike the div's internal wheel/touch scroll, which calls
        // `cx.notify`). Without this, the offset changes but no frame is
        // requested, so the scroll is applied yet never painted until some
        // unrelated event triggers a render — the persistent Page Up/Down lag.
        cx.notify();
    }

    /// Scroll to the top of the document (`end` false, Home) or to the bottom
    /// (`end` true, End). The target offset is clamped to `[-max_offset, 0]`
    /// and `cx.notify` forces a frame so the jump paints immediately (same
    /// reason as `scroll_page`).
    fn scroll_to_end(&mut self, end: bool, cx: &mut Context<Self>) {
        let handle = &self.scroll_handle;
        let max_offset = f32::from(handle.max_offset().y);
        let target = if end { -max_offset } else { 0.0 };
        handle.set_offset(point(px(0.), px(target)));
        cx.notify();
    }

    /// Re-renders from `text`, preserving the current scroll position.
    ///
    /// The markdown re-parse is asynchronous: the new document commits in a
    /// later frame, and the content may momentarily shrink (or the layout
    /// reset) during that commit, which would clamp the scroll offset to zero.
    /// So we capture the outer div's current offset now, set the text, and
    /// keep re-applying the captured offset in `render` until the new
    /// document's size is stable for two consecutive frames.
    fn reload(&mut self, text: &str, cx: &mut Context<Self>) {
        self.scroll_restore_offset = Some(self.scroll_handle.offset().y);
        self.restore_content_height = None;
        self.restore_stable_frames = 0;

        self.view_state
            .update(cx, |state, cx| state.set_text(text, cx));
    }
}

impl Render for MarkdownWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.focus_handle.focus(window, cx);
            self.needs_focus = false;
        }

        // The re-parse is asynchronous and the new document's size settles
        // over a few frames, so the captured offset is re-applied each frame
        // until the content height is stable, then the restore stops so the
        // user can scroll freely.
        if let Some(restore_offset) = self.scroll_restore_offset {
            let content_height = self.scroll_handle.max_offset().y;
            if self.restore_content_height != Some(content_height) {
                // The content size changed (the new document committed, or the
                // content is still settling); restart the stability counter.
                self.restore_content_height = Some(content_height);
                self.restore_stable_frames = 0;
            }
            // Clamp to the current content height: the layout pass clamps to
            // `[-max_offset, 0]` anyway, so this keeps the value we set in range.
            let clamped = f32::from(restore_offset).clamp(-f32::from(content_height), 0.0);
            self.scroll_handle.set_offset(point(px(0.), px(clamped)));
            self.restore_stable_frames += 1;
            if self.restore_stable_frames >= 2 {
                self.scroll_restore_offset = None;
                self.restore_content_height = None;
            }
        }

        // Base directory for resolving relative image sources, captured once
        // per frame (owned so the `'static` resolver closure can capture it).
        let base_dir = self.path.parent().map(PathBuf::from);

        // Center the content column responsively: the scrollable div spans the
        // full window width, and the `MAX_CONTENT_WIDTH` column inside it is
        // inset by symmetric padding computed from the actual viewport width,
        // so it adapts to resizes with no hardcoded window size (the overlay
        // scrollbar ignores the padding and stays at the box's outer edge).
        let viewport_width = f32::from(window.viewport_size().width);
        let content_pad = px(((viewport_width - MAX_CONTENT_WIDTH) * 0.5).max(0.0));

        // The scrollbar MUST be a SIBLING of the scrollable div, not a child:
        // a child is painted inside the div's overflow content-mask (clipped
        // to the viewport) and shifted by the div's scroll offset, which
        // pushes the thumb (computed from the handle's fixed viewport bounds)
        // out of the visible viewport and the clip hides it. As a sibling it
        // is painted at a fixed position on top of the content.
        //
        // `id` is required, not cosmetic: the scroll methods live on
        // `StatefulInteractiveElement`, implemented only for `Stateful<Div>`
        // (a div with an element id), and the id must stay stable across
        // renders for the scroll state to persist.
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("markdown-scroll-view")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    // Page Up/Down scroll the viewport by one page height;
                    // the handler is bound to the window's focus handle so
                    // it fires when the window has keyboard focus.
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, _cx| {
                        match event.keystroke.key.as_str() {
                            "pagedown" => view.scroll_page(true, _cx),
                            "pageup" => view.scroll_page(false, _cx),
                            "end" => view.scroll_to_end(true, _cx),
                            "home" => view.scroll_to_end(false, _cx),
                            _ => {}
                        }
                    }))
                    .child(
                        // Full-width inside the scroll, so its padding insets
                        // the document on all four sides while the scroll
                        // viewport stays edge-to-edge.
                        div()
                            .w_full()
                            .pt(px(TOP_PADDING))
                            .pb(px(BOTTOM_PADDING))
                            .pl(content_pad)
                            .pr(content_pad)
                            .line_height(relative(LINE_HEIGHT))
                            .child(
                                TextView::new(&self.view_state)
                                    .style(self.text_style.clone())
                                    // Non-scrollable: the TextView expands to
                                    // fit all of its content and has no
                                    // internal scrollbar, so the outer div's
                                    // scroll handle owns the scroll.
                                    .scrollable(false)
                                    .w_full()
                                    // A standalone binary has no asset
                                    // loader, so a bare `sample.png` would
                                    // resolve as an embedded resource and
                                    // fail silently; resolve it against the
                                    // markdown file's directory instead.
                                    .image_source(move |uri: &SharedUri| {
                                        let source = uri.as_str();
                                        if source.starts_with("http://")
                                            || source.starts_with("https://")
                                        {
                                            ImageSource::from(uri.clone())
                                        } else if let Some(dir) = base_dir.as_ref() {
                                            let absolute = if Path::new(source).is_absolute() {
                                                PathBuf::from(source)
                                            } else {
                                                dir.join(source)
                                            };
                                            ImageSource::from(absolute)
                                        } else {
                                            // No parent directory: keep the
                                            // default resolution.
                                            ImageSource::from(uri.clone())
                                        }
                                    }),
                            ),
                    ),
            )
            // The visible vertical scrollbar: an `absolute().inset_0()`
            // overlay sibling of the scrollable div, painted after it so it
            // sits on top. It is bound to the same `ScrollHandle`, and
            // `viewport_from_layout()` makes it use its own fixed layout
            // bounds (the full window) as its viewport rather than the
            // handle's, so the thumb paints at the window's right edge and
            // reflects the document's scroll position. `Always` keeps it
            // visible at rest (the default theme mode fades it out after
            // idle).
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::vertical(&self.scroll_handle)
                        .id(("markdown-view-scrollbar", cx.entity_id()))
                        .mode(ScrollbarMode::Always)
                        .viewport_from_layout(),
                ),
            )
    }
}
