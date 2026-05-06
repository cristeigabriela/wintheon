//! Spotlight-style launcher example — exercises most of the public
//! library surface against a real egui front-end, and builds more on
//! top of it.
//!
//! # Build
//!
//! `cargo run --example launcher`
//!
//! The window will appear at startup, and `Shift+Alt+Space` toggles it
//! from anywhere on the system, using global hotkeys.
//!
//! # UX
//!
//! - **Global hotkey** (`Shift+Alt+Space`): toggle the launcher from
//!   any application. Registered via `global-hotkey`; a small polling
//!   thread wakes the egui loop with `request_repaint` so the toggle
//!   is responsive even while hidden.
//! - **Frameless + always-on-top**: no title bar, no resize handles,
//!   floats above other windows. Set in `eframe::NativeOptions`.
//! - **Compact ↔ expanded**: the window is just header + search bar
//!   (~120 px tall) when there's no query; growing to full height
//!   (720 px) the moment the user types. The top edge stays put across
//!   the transition — only the height changes — so the search bar
//!   doesn't jump.
//! - **Auto-focus search**: the search bar grabs keyboard focus the
//!   first frame after the launcher becomes visible, so character keys
//!   land in the query without a click.
//! - **Keyboard navigation**: `↑` / `↓` moves the highlighted result,
//!   `Enter` launches it (then hides), `Escape` hides without
//!   launching. Both Enter and Escape clear the query so the next
//!   summon starts compact, without previous context.
//! - **Color-coded origin chips**: each row tags Desktop / Start Menu
//!   / Windows Apps with a distinct pill so the source is obvious at
//!   a glance.
//!
//! # Wintheon API surface used
//!
//! - [`Gatherer`](wintheon::gather::Gatherer) wires up the three
//!   built-in sources with explicit per-source priorities.
//! - [`WeightedEntryIteratorExt`](wintheon::gather::WeightedEntryIteratorExt)
//!   drives the per-frame ranking. A small `Indexed(usize, &LauncherEntry)`
//!   wrapper implements `AsRef<WeightedEntry>` so the trait works
//!   directly on the launcher's own type — see `compute_ranked_indices`.
//! - [`WeightedEntry::score`](wintheon::gather::WeightedEntry::score)
//!   does the heavy lifting per entry, with the per-entry
//!   [`MatchIndex`](wintheon::gather::MatchIndex) cache making
//!   per-keystroke re-ranking near-free after the first frame.
//! - [`FileEntry::icon`](wintheon::file::FileEntry::icon) +
//!   [`FileIcon::extract_icon_at`](wintheon::file::FileIcon::extract_icon_at)
//!   produce the per-row RGBA pixels, gated through a two-stage cache
//!   described below.
//!
//! # Icon memory: prewarm + on-disk cache
//!
//! - **Disk cache** ([`icon_cache`]): RGBA bytes live in `%TEMP%`
//!   prefixed with an 8-byte source-mtime header. Reads are mmap-backed;
//!   stale entries (source `mtime` changed since the cache write)
//!   self-invalidate.
//! - **Startup prewarm** ([`spawn_prewarm_worker`]): a background
//!   thread walks every entry once at launch and ensures the disk
//!   cache is current. First run does the full extraction sweep;
//!   subsequent runs skip almost everything via [`icon_cache::is_cached`].
//! - **Lazy synchronous upload** ([`LauncherEntry::ensure_texture`]):
//!   the first time a row renders visibly, it loads its GPU texture
//!   straight from the disk cache. Cache hits are sub-millisecond, and
//!   prewarm makes the cache warm by the time the user types — so the
//!   render-time call doesn't need a worker thread or spinner.
//!
//! # Tracing
//!
//! `tracing-subscriber` is wired up at startup, defaulting to `info`.
//! Run with `RUST_LOG=wintheon=debug cargo run --example launcher` to
//! see why a `.lnk` failed to resolve, why an icon came back blank,
//! how long each source's scan took, etc. — the library's `debug`
//! events are exactly the ones useful for diagnosing real-world
//! Windows weirdness.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use eframe::egui;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
use wintheon::file::{FileEntry, FileIcon, IconSize, Priority};
use wintheon::gather::{Gatherer, Origin, WeightedEntry, WeightedEntryIteratorExt};

/// Side length we extract icons at.
const ICON: IconSize = IconSize::Custom(64);
/// Draw at half the size of the resolution, looks pretty crisp!
const ICON_PX: f32 = ICON.pixels() as f32 / 2.0;

/// Window width — same in compact and expanded modes.
const WIN_W: f32 = 580.0;

/// Compact height: just enough for the header + search bar. What the
/// launcher shows when there's no query (idle / freshly-summoned).
const WIN_H_COMPACT: f32 = 120.0;

/// Expanded height: kicks in as soon as the user starts typing. The
/// list scrolls when matches exceed what fits.
const WIN_H: f32 = 720.0;

/// Hard cap on a single card's width. Cards stop stretching past this
/// even if the available content area is wider — keeps the layout
/// readable on hypothetical wider windows.
const MAX_CARD_WIDTH: f32 = 540.0;

/// Pixels from the top of the screen to the launcher's top edge.
/// Spotlight-style — "high up" rather than dead-center, so the window
/// can grow downward without ever needing to move the search bar.
const WIN_TOP_OFFSET: f32 = 120.0;

/// On-disk RGBA cache for extracted icons, indexed by source path.
///
/// Files live under `%TEMP%\wintheon-launcher-icons\` as
/// `{hash}_{size}.rgba`, with the layout:
///
/// ```text
/// [mtime_ns: u64 LE][rgba: size*size*4 bytes]
/// ```
///
/// The 8-byte header carries the **source file's `mtime` at the moment
/// the cache entry was written**, and the read paths compare it against
/// the live source `mtime` to invalidate stale entries. So if a user
/// updates an installed app, the `.lnk` or target `.exe` gets a new
/// `mtime`, so the next read sees the mismatch, treats it as a miss,
/// and re-extracts the (potentially new) icon.
///
/// Reads to the cache go through `mmap-io` so the OS can page-fault icons in lazily.
/// Writes are plain `std::fs::write`. The cache survives across runs of the launcher,
/// so a cold start of the second invocation skips Win32 GDI extraction entirely.
mod icon_cache {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::UNIX_EPOCH;

    use mmap_io::mmap::MemoryMappedFile;

    /// Bytes of mtime header that prefix the RGBA payload. See module
    /// doc for the layout.
    const HEADER_LEN: u64 = 8;

    static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

    /// Lazily create the cache directory under the system temp dir.
    ///
    /// `create_dir_all` ignores "already exists" errors so the `OnceLock`
    /// path is set unconditionally.
    fn cache_dir() -> &'static Path {
        CACHE_DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join("wintheon-launcher-icons");
            let _ = std::fs::create_dir_all(&dir);
            dir
        })
    }

    /// `{hash}_{size}.rgba`: the size in the filename means changing
    /// the launcher's `IconSize` constant invalidates only old-size
    /// entries; new-size icons get fresh files alongside.
    fn cache_path(source: &Path, size: u32) -> PathBuf {
        let mut h = DefaultHasher::new();
        source.to_string_lossy().hash(&mut h);
        cache_dir().join(format!("{:016x}_{size}.rgba", h.finish()))
    }

    fn rgba_byte_len(size: u32) -> u64 {
        (size as u64) * (size as u64) * 4
    }

    /// Source file's mtime as nanoseconds since UNIX_EPOCH. `None` if
    /// the path can't be stat'd (deleted, permission error) or its
    /// mtime messed up thoroughly.
    ///
    /// A `None` here is propagated as a cache miss / no-store, which simply
    /// means we always extract for that file, so it's never incorrect.
    fn source_stamp(source: &Path) -> Option<u64> {
        let mtime = std::fs::metadata(source).ok()?.modified().ok()?;
        Some(mtime.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64)
    }

    /// Read just the 8-byte mtime header from a cache file. Used by
    /// [`is_cached`] to avoid the mmap setup that [`try_load`] does.
    fn read_header(path: &Path) -> Option<u64> {
        let mut f = std::fs::File::open(path).ok()?;
        let mut buf = [0u8; HEADER_LEN as usize];
        f.read_exact(&mut buf).ok()?;
        Some(u64::from_le_bytes(buf))
    }

    /// Map the cache file for `source` and return its bytes. `None` for
    /// any failure mode: missing file, wrong size, source mtime
    /// changed since the entry was written, I/O error. Caller treats it
    /// as a miss and falls through to live extraction.
    pub fn try_load(source: &Path, size: u32) -> Option<Vec<u8>> {
        let path = cache_path(source, size);
        let mmap = MemoryMappedFile::open_ro(&path).ok()?;
        if mmap.len() != HEADER_LEN + rgba_byte_len(size) {
            return None;
        }
        let header: [u8; HEADER_LEN as usize] =
            mmap.as_slice(0, HEADER_LEN).ok()?.try_into().ok()?;
        if u64::from_le_bytes(header) != source_stamp(source)? {
            return None;
        }
        Some(
            mmap.as_slice(HEADER_LEN, rgba_byte_len(size))
                .ok()?
                .to_vec(),
        )
    }

    /// Cheap "is there a current, mtime-matched entry for this source?"
    /// probe. Used by the prewarm pass to skip extractions whose
    /// results are already on disk from a previous run.
    pub fn is_cached(source: &Path, size: u32) -> bool {
        let path = cache_path(source, size);
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        if meta.len() != HEADER_LEN + rgba_byte_len(size) {
            return false;
        }
        let (Some(stored), Some(current)) = (read_header(&path), source_stamp(source)) else {
            return false;
        };
        stored == current
    }

    /// Write freshly-extracted RGBA bytes back to the cache, prefixed
    /// with the source's current mtime so future reads can verify.
    //
    // Fails silently, means you just don't get a cache for the file if that happens.
    pub fn store(source: &Path, size: u32, rgba: &[u8]) {
        let Some(stamp) = source_stamp(source) else {
            return;
        };
        let path = cache_path(source, size);
        let mut buf = Vec::with_capacity(HEADER_LEN as usize + rgba.len());
        buf.extend_from_slice(&stamp.to_le_bytes());
        buf.extend_from_slice(rgba);
        let _ = std::fs::write(path, buf);
    }
}

/// Position the launcher horizontally centered, anchored
/// `WIN_TOP_OFFSET` pixels below the top of the primary monitor. The
/// vertical position is independent of the launcher's height — that
/// way, on a compact↔expanded transition we only resize, the top edge
/// (and the search bar with it) stays put.
fn launcher_position() -> egui::Pos2 {
    // SAFETY: `GetSystemMetrics` is callable from any thread, never
    // fails, and reads no out-parameters — pure read of OS state.
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) as f32 };
    egui::Pos2::new(((screen_w - WIN_W) / 2.0).max(0.0), WIN_TOP_OFFSET)
}

fn main() -> eframe::Result<()> {
    // Route library `tracing` events to stderr. Defaults to `info` when
    // `RUST_LOG` is unset.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let entries = collect_entries();

    // Kick off background prewarm: walk every entry, extract the icon,
    // stash it in the disk cache.
    //
    // The actual GDI extraction for icons happens off-thread and races
    // benignly with render-time `ensure_texture` calls.
    let prewarm: Vec<(PathBuf, FileIcon)> = entries
        .iter()
        .filter_map(|e| {
            let icon = e.weighted.entry.icon().ok()?;
            Some((e.launch_target().to_path_buf(), icon))
        })
        .collect();
    spawn_prewarm_worker(prewarm);

    // Register the global Shift+Alt+Space hotkey before creating the
    // window so we know it's grabbed at startup. The manager has to
    // outlive the app, so we hand it to `Launcher` which keeps it alive.
    let hotkey_manager =
        GlobalHotKeyManager::new().expect("failed to initialize the global-hotkey manager");
    let hotkey = HotKey::new(Some(Modifiers::SHIFT | Modifiers::ALT), Code::Space);
    hotkey_manager
        .register(hotkey)
        .expect("failed to register Shift+Alt+Space (already taken?)");
    let hotkey_id = hotkey.id();

    // Channel from the polling thread to the egui event loop: each
    // event becomes a `()` ping that the main thread treats as a toggle.
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<()>();

    // Start in compact mode (no query yet).
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WIN_W, WIN_H_COMPACT])
            .with_position(launcher_position())
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "wintheon example launcher",
        options,
        Box::new(move |cc| {
            // Slightly more breathing room between widgets than the
            // default. Card backgrounds and hover colors are picked from
            // the active theme via `ui.style().visuals.*` so the launcher
            // looks right in both light and dark mode.
            cc.egui_ctx.global_style_mut(|style| {
                style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            });

            // Spawn a thread that blocks on the global-hotkey channel
            // and pings the launcher every time our hotkey fires. The
            // `request_repaint` call wakes the egui loop even when the
            // window is hidden, so the toggle is responsive.
            //
            // `global-hotkey` emits a separate event for press *and*
            // release of the same key combo. Filtering on
            // `HotKeyState::Pressed` keeps the toggle to one signal
            // per actual user press.
            let egui_ctx = cc.egui_ctx.clone();
            thread::spawn(move || {
                let receiver = GlobalHotKeyEvent::receiver();
                while let Ok(event) = receiver.recv() {
                    if event.id != hotkey_id || event.state != HotKeyState::Pressed {
                        continue;
                    }
                    if hotkey_tx.send(()).is_err() {
                        break;
                    }
                    egui_ctx.request_repaint();
                }
            });

            Ok(Box::new(Launcher::new(entries, hotkey_rx, hotkey_manager)))
        }),
    )
}

fn collect_entries() -> Vec<LauncherEntry> {
    let gatherer = Gatherer::new()
        .with_desktop(Priority(1.0))
        .with_start_menu(Priority(1.5))
        .with_windows_apps(Priority(2.0));

    gatherer
        .scan()
        .filter_map(|r| r.ok())
        .enumerate()
        .map(|(i, w)| LauncherEntry::new(i, w))
        .collect()
}

/// English-translation version-info subset used by the row renderer.
#[derive(Default, Clone)]
struct EntryMeta {
    company: Option<String>,
    file_version: Option<String>,
    product_version: Option<String>,
    original_filename: Option<String>,
    copyright: Option<String>,
}

impl EntryMeta {
    fn from_entry(entry: &dyn FileEntry) -> Self {
        entry
            .version_info()
            .ok()
            .and_then(|info| info.english().cloned())
            .map(|fi| Self {
                company: fi.company_name,
                file_version: fi.file_version,
                product_version: fi.product_version,
                original_filename: fi.original_filename,
                copyright: fi.legal_copyright,
            })
            .unwrap_or_default()
    }
}

/// Background pass that walks every entry once at startup and ensures
/// each has a fresh [`icon_cache`] entry on disk, so that
/// [`LauncherEntry::ensure_texture`] only ever does a fast cache read
/// at render time.
fn spawn_prewarm_worker(work: Vec<(PathBuf, FileIcon)>) {
    thread::spawn(move || {
        let size = ICON.pixels();
        let total = work.len();
        let started = Instant::now();
        let mut extracted = 0usize;
        let mut already_cached = 0usize;
        let mut failed = 0usize;
        for (cache_key, icon) in work {
            if icon_cache::is_cached(&cache_key, size) {
                already_cached += 1;
                continue;
            }
            match icon.extract_icon_at(ICON) {
                Some(rgba) => {
                    icon_cache::store(&cache_key, size, &rgba);
                    extracted += 1;
                }
                None => failed += 1,
            }
        }
        tracing::info!(
            total,
            extracted,
            already_cached,
            failed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "icon prewarm complete"
        );
    });
}

/// Per-entry GPU texture state. Loaded lazily on first visible render
/// via [`LauncherEntry::ensure_texture`]; stays loaded for the rest of
/// the session (no eviction, peak GPU footprint is bounded by the
/// number of entries the user actually scrolls through).
#[derive(Default)]
enum TextureSlot {
    /// First-render state — no upload attempted yet.
    #[default]
    Unloaded,
    /// Cache miss + live extraction also failed (e.g. file has no
    /// shell icon). We don't retry within the session.
    Failed,
    /// RGBA uploaded to GPU; `ui.image` paints from this handle.
    Loaded(egui::TextureHandle),
}

/// One scan result plus per-entry caches.
struct LauncherEntry {
    /// Stable position in `Launcher::entries`. Used as a unique key
    /// when uploading the texture (egui requires a `&str` name).
    idx: usize,
    weighted: WeightedEntry,
    /// Rendering metadata (English version-info strings, not lowercased).
    metadata: OnceLock<EntryMeta>,
    /// GPU texture slot. `RefCell` so `ensure_texture` can mutate
    /// from inside the shared-ref render walk.
    texture: RefCell<TextureSlot>,
}

/// Lets [`WeightedEntryIteratorExt`] drive ranking off `&LauncherEntry`
/// directly.
///
/// `.sorted_by_score(query)` returns a `Vec<&LauncherEntry>`
/// without any unwrapping.
impl AsRef<WeightedEntry> for LauncherEntry {
    fn as_ref(&self) -> &WeightedEntry {
        &self.weighted
    }
}

impl LauncherEntry {
    fn new(idx: usize, weighted: WeightedEntry) -> Self {
        Self {
            idx,
            weighted,
            metadata: OnceLock::new(),
            texture: RefCell::new(TextureSlot::Unloaded),
        }
    }

    fn metadata(&self) -> &EntryMeta {
        self.metadata
            .get_or_init(|| EntryMeta::from_entry(self.weighted.entry.as_ref()))
    }

    /// What we actually hand to the shell on click: the link path for
    /// shortcuts/reparse stubs, the file path for plain entries.
    fn launch_target(&self) -> &Path {
        self.weighted
            .entry
            .link_path()
            .unwrap_or_else(|| self.weighted.entry.path())
    }

    /// Load the row's texture into GPU memory.
    ///
    /// Best case scenario: disk cache is hit, CPU-side is mmap-backed.
    /// Worst case scenario: we do icon extraction now.
    ///
    /// NOP after load.
    fn ensure_texture(&self, ctx: &egui::Context) {
        let mut slot = self.texture.borrow_mut();
        if !matches!(*slot, TextureSlot::Unloaded) {
            return;
        }
        let key = self.launch_target();
        let size = ICON.pixels();
        let rgba = icon_cache::try_load(key, size).or_else(|| {
            let icon = self.weighted.entry.icon().ok()?;
            let extracted = icon.extract_icon_at(ICON)?;
            icon_cache::store(key, size, &extracted);
            Some(extracted)
        });
        *slot = match rgba {
            Some(bytes) => {
                let px = ICON.pixels() as usize;
                let img = egui::ColorImage::from_rgba_unmultiplied([px, px], &bytes);
                let tex = ctx.load_texture(
                    format!("launcher_icon_{}", self.idx),
                    img,
                    egui::TextureOptions::LINEAR,
                );
                TextureSlot::Loaded(tex)
            }
            None => TextureSlot::Failed,
        };
    }

    /// Cheap clone of the loaded texture handle (Arc inside), or `None`
    /// if the slot isn't `Loaded`. The clone lets `render_row` drop the
    /// `RefCell` borrow before recursing into closures.
    fn texture_handle(&self) -> Option<egui::TextureHandle> {
        match &*self.texture.borrow() {
            TextureSlot::Loaded(t) => Some(t.clone()),
            _ => None,
        }
    }
}

struct Launcher {
    entries: Vec<LauncherEntry>,
    query: String,
    /// Index into the *currently-ranked* slice of entries — the row the
    /// user is highlighting with ↑/↓. Reset to 0 whenever the query
    /// changes (most-relevant result floats to the top).
    selected: usize,
    /// `true` when the launcher is on screen. Toggled by the global
    /// hotkey and by Escape / Enter.
    visible: bool,
    /// Set the next time the launcher becomes visible — causes the
    /// search bar to grab focus on the following frame so a-z keys go
    /// straight to the query without a click.
    request_focus: bool,
    /// Last applied "compact mode" state — `true` when query is empty
    /// and we've shrunk the window to header + search bar only. Used
    /// to detect transitions and only send resize commands on change.
    last_compact: bool,
    hotkey_rx: mpsc::Receiver<()>,
    /// Held to keep the global hotkey registered for the app's lifetime
    /// — drops automatically when `Launcher` is dropped on shutdown.
    _hotkey_manager: GlobalHotKeyManager,
}

impl Launcher {
    fn new(
        entries: Vec<LauncherEntry>,
        hotkey_rx: mpsc::Receiver<()>,
        hotkey_manager: GlobalHotKeyManager,
    ) -> Self {
        Self {
            entries,
            query: String::new(),
            selected: 0,
            visible: true,
            request_focus: true,
            // Initial size matches the compact viewport above; record it
            // so the first frame doesn't issue a redundant resize.
            last_compact: true,
            hotkey_rx,
            _hotkey_manager: hotkey_manager,
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.visible = true;
        self.request_focus = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.visible = false;
        // Reset state so the next summon starts fresh.
        self.query.clear();
        self.selected = 0;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn toggle_visibility(&mut self, ctx: &egui::Context) {
        if self.visible {
            self.hide(ctx);
        } else {
            self.show(ctx);
        }
    }

    /// Resize the window when the launcher transitions between compact
    /// (no query, header + search only) and expanded (showing the
    /// ranked results list).
    fn apply_compact_state(&mut self, ctx: &egui::Context) {
        let compact = self.query.is_empty();
        if compact == self.last_compact {
            return;
        }
        self.last_compact = compact;
        let height = if compact { WIN_H_COMPACT } else { WIN_H };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(WIN_W, height)));
    }
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 1. Drain hotkey pings (toggle visibility per ping).
        while self.hotkey_rx.try_recv().is_ok() {
            self.toggle_visibility(&ctx);
        }

        // 2. Consume keyboard navigation BEFORE the search field is
        //    rendered, so `TextEdit` doesn't swallow the events.
        let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

        if escape {
            self.hide(&ctx);
            return;
        }

        // 3. Compute ranking as owned `Vec<usize>` (indices into
        //    `self.entries`) — owned so it doesn't borrow `self`,
        //    which lets us pass `&mut self` to `render` below.
        let ranked = self.compute_ranked_indices();

        // 4. Apply navigation. Selection clamps to the ranked slice's
        //    bounds — handles the "query just shrank the list under us"
        //    case automatically.
        if !ranked.is_empty() {
            if down {
                self.selected = (self.selected + 1).min(ranked.len() - 1);
            }
            if up {
                self.selected = self.selected.saturating_sub(1);
            }
            if self.selected >= ranked.len() {
                self.selected = ranked.len() - 1;
            }
        } else {
            self.selected = 0;
        }

        // 5. Enter launches the selected row, then hides. `hide()`
        //    clears the query, so the next show is in compact mode.
        if enter {
            if let Some(&entry_idx) = ranked.get(self.selected) {
                let target = self.entries[entry_idx].launch_target().to_path_buf();
                spawn(&target);
            }
            self.hide(&ctx);
            return;
        }

        // 6. Resize on compact↔expanded transitions. Done here rather
        //    than inside `render` so commands fire even if something
        //    in `render` short-circuits.
        self.apply_compact_state(&ctx);

        egui::Frame::default()
            .inner_margin(egui::Margin::symmetric(20, 18))
            .show(ui, |ui| self.render(ui, &ctx, &ranked));
    }
}

/// Carries an entry alongside its position in `Launcher::entries` so
/// that ranking can be driven by the library's
/// [`WeightedEntryIteratorExt`] trait while still recovering the
/// original index for index-based render lookups.
///
/// Implementing `AsRef<WeightedEntry>` is the only thing the trait
/// requires of a custom item type; the rest comes for free.
struct Indexed<'a>(usize, &'a LauncherEntry);

impl AsRef<WeightedEntry> for Indexed<'_> {
    fn as_ref(&self) -> &WeightedEntry {
        &self.1.weighted
    }
}

impl Launcher {
    /// Score every entry, drop non-matches, sort highest-score-first,
    /// and return the surviving indices into `self.entries`.
    fn compute_ranked_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| Indexed(i, e))
            .sorted_by_score(&self.query)
            .into_iter()
            .map(|Indexed(i, _)| i)
            .collect()
    }

    fn render(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, ranked: &[usize]) {
        // Header
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("wintheon").size(22.0).strong());
            ui.label(
                egui::RichText::new(format!("· {} entries", self.entries.len()))
                    .size(13.0)
                    .weak(),
            );
        });
        ui.add_space(10.0);

        // Search: auto-focus on first frame after becoming visible so
        // the user can start typing without clicking. Resetting
        // `selected = 0` on every query change keeps the highlight
        // pinned to the top result.
        let prev_query_len = self.query.len();
        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .desired_width(f32::INFINITY)
                .hint_text("Search…")
                .font(egui::FontId::proportional(18.0))
                .margin(egui::Margin::symmetric(10, 8)),
        );
        if self.request_focus {
            search_resp.request_focus();
            self.request_focus = false;
        }
        if search_resp.changed() || self.query.len() != prev_query_len {
            self.selected = 0;
        }

        // No query → compact mode, header + search only. The window is
        // resized to match by `apply_compact_state` in the update loop.
        if self.query.is_empty() {
            return;
        }

        ui.add_space(12.0);

        // List
        let selected = self.selected;
        let entries = &self.entries;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ranked.is_empty() {
                    self.render_empty_state(ui);
                    return;
                }
                for (row_idx, &entry_idx) in ranked.iter().enumerate() {
                    render_row(row_idx, &entries[entry_idx], row_idx == selected, ui, ctx);
                    ui.add_space(6.0);
                }
            });
    }

    fn render_empty_state(&self, ui: &mut egui::Ui) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("No matches").size(18.0).weak());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("for {:?}", self.query))
                    .size(13.0)
                    .weak()
                    .italics(),
            );
        });
    }
}

fn render_row(
    row_idx: usize,
    entry: &LauncherEntry,
    selected: bool,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
) {
    // Visibility-gated texture upload: only entries that are about to
    // paint pixels do the (cheap, cache-fast) load. `next_widget_position`
    // gives the row's top-left before we render; pairing it with a
    // worst-case row height (100 px) gives an approximate rect that
    // `is_rect_visible` can clip against the scroll viewport.
    //
    // Selected rows always count as visible — `scroll_to_me` will pull
    // them into view next frame, and we want their texture ready.
    let approx_rect =
        egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(MAX_CARD_WIDTH, 100.0));
    if selected || ui.is_rect_visible(approx_rect) {
        entry.ensure_texture(ctx);
    }

    // Snapshot every value we need before borrowing into the closure tree.
    let texture = entry.texture_handle();
    let name = entry.weighted.entry.display_name();
    let meta = entry.metadata().clone();
    let origin = entry.weighted.origin.clone();
    let full_path = entry.weighted.entry.path().display().to_string();
    let launch_target = entry.launch_target().to_path_buf();

    // Read the previous frame's response for this row so we can paint a
    // hover background underneath the content. One frame of lag is
    // imperceptible at 60 fps and avoids the two-pass layout dance.
    let row_id = ui.id().with("launcher_row").with(row_idx);
    let hovered = ctx.read_response(row_id).is_some_and(|r| r.hovered());

    // Card background priority: keyboard-selected → hovered → resting.
    // The "active" widget fill is the most prominent; "hovered" is
    // intermediate; resting falls back to the panel-adjacent
    // `faint_bg_color`. All theme-aware.
    let visuals = &ui.style().visuals;
    let card_fill = if selected {
        visuals.widgets.active.weak_bg_fill
    } else if hovered {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.faint_bg_color
    };
    let frame = egui::Frame::default()
        .fill(card_fill)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .corner_radius(10.0);

    let frame_response = frame
        .show(ui, |ui| {
            // Cap card width: at most `MAX_CARD_WIDTH`, otherwise the
            // available content area. Setting both min and max to the
            // same value pins the card to that width, so the layout
            // stays consistent regardless of scrollbar appearance or
            // future window-size changes.
            let card_width = ui.available_width().min(MAX_CARD_WIDTH);
            ui.set_min_width(card_width);
            ui.set_max_width(card_width);
            ui.horizontal(|ui| {
                // Force the row to claim at least the icon's height so
                // egui's cross-axis `Align::Center` (default for
                // `horizontal()`) can vertically center the label block
                // against the icon when the labels stack to less.
                ui.set_min_height(ICON_PX);
                render_icon_slot(ui, texture.as_ref());
                ui.add_space(12.0);

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&name)
                                .size(16.0)
                                .strong()
                                .color(ui.style().visuals.strong_text_color()),
                        );
                        ui.add_space(8.0);
                        origin_chip(ui, &origin);
                    });

                    let mut tail: Vec<String> = Vec::new();
                    if let Some(c) = &meta.company {
                        tail.push(c.clone());
                    }
                    if let Some(v) = meta
                        .file_version
                        .as_deref()
                        .or(meta.product_version.as_deref())
                    {
                        tail.push(format!("v{v}"));
                    }
                    if let Some(o) = &meta.original_filename {
                        tail.push(o.clone());
                    }
                    if !tail.is_empty() {
                        ui.label(egui::RichText::new(tail.join("  ·  ")).size(12.0).weak());
                    }

                    if let Some(cr) = meta.copyright.as_deref() {
                        ui.label(egui::RichText::new(cr).size(11.0).weak().italics());
                    }

                    ui.label(
                        egui::RichText::new(&full_path)
                            .size(11.0)
                            .monospace()
                            .color(ui.style().visuals.weak_text_color()),
                    );
                });
            });
        })
        .response;

    // Make the entire card a click target. Cursor changes to a hand on
    // hover; clicking anywhere on the row launches the entry.
    let click = ui
        .interact(frame_response.rect, row_id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if click.clicked() {
        spawn(&launch_target);
    }

    // Keep the keyboard-selected row visible as the user arrows past
    // the viewport. `scroll_to_me(Center)` smoothly recenters the row.
    if selected {
        click.scroll_to_me(Some(egui::Align::Center));
    }
}

/// Render the icon slot — fixed `ICON_PX` × `ICON_PX` regardless of
/// load state, so the row layout stays stable. `Some` paints the
/// texture; `None` (still loading or extraction failed) leaves blank
/// space so the labels don't shift around.
fn render_icon_slot(ui: &mut egui::Ui, texture: Option<&egui::TextureHandle>) {
    let size = egui::vec2(ICON_PX, ICON_PX);
    match texture {
        Some(tex) => {
            ui.image((tex.id(), size));
        }
        None => {
            ui.allocate_space(size);
        }
    }
}

/// Pill-shaped, color-coded chip for an [`Origin`] — gives each source
/// a distinct visual identity in the row header.
fn origin_chip(ui: &mut egui::Ui, origin: &Origin) {
    let (bg, label): (egui::Color32, &str) = match origin {
        Origin::Desktop => (egui::Color32::from_rgb(60, 110, 200), "Desktop"),
        Origin::StartMenu => (egui::Color32::from_rgb(60, 150, 90), "Start Menu"),
        Origin::WindowsApps => (egui::Color32::from_rgb(150, 90, 200), "Windows Apps"),
        Origin::Custom(label) => (egui::Color32::DARK_GRAY, label.as_ref()),
        // `Origin` is `#[non_exhaustive]`; future variants display
        // under a neutral chip until we wire dedicated colors.
        _ => (egui::Color32::DARK_GRAY, "?"),
    };
    egui::Frame::default()
        .fill(bg)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(11.0)
                    .strong()
                    .color(egui::Color32::WHITE),
            );
        });
}

/// Hand the path to the shell so it routes `.lnk`, AppExec stubs, and
/// plain executables uniformly.
fn spawn(path: &Path) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .spawn();
}
