//! File watching via `notify` with a smol async channel for debounced reloads.

use std::path::{Path, PathBuf};

use anyhow::Result;
use gpui_kit::base::async_util::{Receiver, unbounded};
use notify::{RecursiveMode, Watcher, recommended_watcher};

/// Holds a `notify::RecommendedWatcher` alive; dropping it stops the watch.
pub struct FileWatcher {
    // Kept so the watcher (and its background thread) lives as long as the
    // view that owns it.
    #[allow(dead_code)]
    watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    /// Creates a watcher for `path` that forwards file-modification events as
    /// `PathBuf` values into a new unbounded channel.
    ///
    /// The returned `Receiver` yields the watched path once per modify event;
    /// the caller is responsible for debouncing.
    pub fn new(path: &Path) -> Result<(Self, Receiver<PathBuf>)> {
        let (sender, receiver) = unbounded::<PathBuf>();
        let watched = path.to_path_buf();
        let mut watcher = recommended_watcher(
            move |result: std::result::Result<notify::Event, notify::Error>| {
                match result {
                    Ok(event) if event.kind.is_modify() => {
                        // Best-effort: a full channel simply drops the duplicate event.
                        let _ = sender.try_send(watched.clone());
                    }
                    _ => {}
                }
            },
        )
        .map_err(|err| anyhow::anyhow!("failed to create file watcher: {err}"))?;
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|err| anyhow::anyhow!("failed to watch {path:?}: {err}"))?;
        Ok((Self { watcher }, receiver))
    }
}
