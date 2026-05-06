//! Discover and inspect launchable Windows file entries.
//!
//! `wintheon` walks well-known locations (Desktop, Start Menu, Microsoft
//! Store `WindowsApps`) — or any [`Source`](gather::Source) you provide —
//! and yields a stream of typed [`FileEntry`](file::FileEntry)s. Each
//! entry exposes its resolved target path, shell icon, full
//! `VS_VERSIONINFO` resource, and a heuristic ranking priority.
//!
//! # Example
//!
//! Top 10 launchable entries matching a query, each with its 64×64
//! shell icon encoded as PNG bytes:
//!
//! ```rust,no_run
//! use wintheon::file::{FileEntry, IconSize, Priority};
//! use wintheon::gather::{Gatherer, WeightedEntryIteratorExt};
//!
//! let gatherer = Gatherer::new()
//!     .with_desktop(Priority(1.0))
//!     .with_start_menu(Priority(1.5))
//!     .with_windows_apps(Priority(2.0));
//!
//! // Filter + rank by combined source × entry × query relevance, in one call.
//! let top = gatherer
//!     .scan()
//!     .filter_map(|r| r.ok())
//!     .sorted_by_score("chrome");
//!
//! for weighted in top.iter().take(10) {
//!     let entry = &weighted.entry;
//!     let icon_png = entry
//!         .icon()
//!         .ok()
//!         .and_then(|fi| fi.extract_icon_as_png_at(IconSize::Custom(64)));
//!     println!(
//!         "[{}] {}  ({} bytes of PNG)",
//!         weighted.origin,
//!         entry.display_name(),
//!         icon_png.as_deref().map(<[u8]>::len).unwrap_or(0),
//!     );
//! }
//! ```
//!
//! Pass an empty needle (`""`) to `sorted_by_score` to rank by priority
//! alone — empty query is a neutral 1.0 multiplier.
//!
//! # Launcher example
//!
//! `cargo run --example launcher` opens a Spotlight-style search bar
//! built on this crate plus `eframe` — a working reference for how the
//! pieces fit together in a real UI.
//!
//! ## Wintheon APIs it uses, and how
//!
//! - [`Gatherer`](gather::Gatherer): wired up at startup with Desktop,
//!   Start Menu, and Windows Apps at distinct priorities so Microsoft
//!   Store apps outrank Start Menu shortcuts which outrank loose
//!   Desktop files.
//! - [`WeightedEntryIteratorExt::sorted_by_score`](gather::WeightedEntryIteratorExt::sorted_by_score):
//!   called every keystroke to re-rank the entire entry list against
//!   the current query. The per-entry
//!   [`MatchIndex`](gather::MatchIndex) cache makes it near-free after
//!   the first frame.
//! - [`FileEntry::icon`](file::FileEntry::icon) +
//!   [`FileIcon::extract_icon_at`](file::FileIcon::extract_icon_at):
//!   produce 64×64 RGBA bytes for each row's GPU texture.
//! - [`FileEntry::version_info`](file::FileEntry::version_info):
//!   pulled into the row renderer to surface the company name, file
//!   version, original filename, and copyright string as a metadata
//!   line under each entry's title.
//! - [`FileEntry::link_path`](file::FileEntry::link_path): preferred
//!   over `path()` when handing the launch target to the shell, so
//!   `.lnk` shortcuts and `AppExec` stubs route through their wrappers.
//!
//! ## Extras layered on top
//!
//! UX:
//!
//! - Global `Shift+Alt+Space` hotkey (`global-hotkey`) toggles the
//!   window from any application, with `request_repaint` to wake the
//!   egui loop while hidden.
//! - Frameless, always-on-top viewport with no resize handles.
//! - Compact ↔ expanded resize: header + search bar only when the
//!   query is empty; full 720 px list the moment the user types. Top
//!   edge stays put across the transition.
//! - Spotlight-style window positioning anchored 120 px below the top
//!   of the primary monitor; horizontally centered via
//!   `GetSystemMetrics(SM_CXSCREEN)`.
//! - ↑ / ↓ navigation, Enter to launch, Escape to dismiss; both Enter
//!   and Escape clear the query. Auto-focus on summon so character
//!   keys land in the search bar.
//! - Color-coded origin chips, theme-aware card backgrounds (selected
//!   vs hovered vs resting), version-info badges per row.
//!
//! Performance:
//!
//! - On-disk RGBA cache under `%TEMP%` with an 8-byte source-`mtime`
//!   header for self-invalidation. Reads go through `mmap-io` for
//!   zero-copy + lazy paging.
//! - Background prewarm thread at startup walks every entry once and
//!   ensures the disk cache is current — first run extracts everything
//!   off the UI thread; subsequent runs are near-instant.
//! - Lazy, visibility-gated texture upload at render time: rows the
//!   user can actually see load straight from the (warm) disk cache
//!   into a GPU texture, no spinner or worker thread needed.
//!
//! # Module overview
//!
//! - [`mod@file`] — domain types ([`FileEntry`](file::FileEntry),
//!   [`FileIcon`](file::FileIcon), [`FileVersionInfo`](file::FileVersionInfo),
//!   [`Priority`](file::Priority)) and the three built-in entry kinds.
//! - [`gather`] — discovery and ranking: the [`Source`](gather::Source)
//!   trait + built-in sources, the [`Gatherer`](gather::Gatherer)
//!   builder, [`MatchIndex`](gather::MatchIndex)-backed query relevance,
//!   and the [`WeightedEntryIteratorExt`](gather::WeightedEntryIteratorExt)
//!   extension trait (`.score(q)` / `.sorted_by_score(q)`).
//!
//! # Tracing
//!
//! Internally uses the [`tracing`] crate at `debug` (silently-swallowed
//! Win32 failures, per-source scan timing) and `trace` (per-entry
//! classification, dedup decisions). Install a subscriber such as
//! `tracing-subscriber` and filter on `wintheon=debug` to see them.
//!
//! [`tracing`]: https://docs.rs/tracing

pub mod file;
pub mod gather;
#[doc(hidden)]
pub mod win;
