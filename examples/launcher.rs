//! Minimal launcher example — gathers entries via `wintheon::gather::Gatherer`,
//! renders each with its shell icon and version-info metadata, and launches
//! the highlighted entry through the shell.

use std::path::Path;
use std::process::Command;

use eframe::egui;
use wintheon::file::{FileEntry, ICON_SIZE, Priority};
use wintheon::gather::{Gatherer, WeightedEntry};

/// Pixel side length of each rendered icon. Matches the raw size we
/// extract via `FileIcon::extract_icon`.
const ICON_PX: f32 = 40.0;

fn main() -> eframe::Result<()> {
    let entries = collect_entries();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([580.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "wintheon — launcher",
        options,
        Box::new(|_cc| Ok(Box::new(Launcher::new(entries)))),
    )
}

fn collect_entries() -> Vec<LauncherEntry> {
    Gatherer::new()
        .with_desktop(Priority(1.0))
        .with_start_menu(Priority(1.5))
        .with_windows_apps(Priority(1.0))
        .scan()
        .filter_map(|r| r.ok())
        .map(LauncherEntry::new)
        .collect()
}

/// English-translation version-info pulled in one read.
#[derive(Default, Clone)]
struct EntryMeta {
    description: Option<String>,
    company: Option<String>,
    file_version: Option<String>,
    product_name: Option<String>,
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
                description: fi.file_description,
                company: fi.company_name,
                file_version: fi.file_version,
                product_name: fi.product_name,
                product_version: fi.product_version,
                original_filename: fi.original_filename,
                copyright: fi.legal_copyright,
            })
            .unwrap_or_default()
    }
}

/// One scan result plus its lazily-populated derived fields. Each entry
/// owns its caches, so there's no risk of two entries with the same
/// resolved target colliding through a shared `HashMap`.
struct LauncherEntry {
    weighted: WeightedEntry,
    display_name: Option<String>,
    metadata: Option<EntryMeta>,
    /// Outer `Option`: have we tried? Inner `Option`: did extraction succeed?
    icon: Option<Option<egui::TextureHandle>>,
    /// Lowercased corpus of every searchable string for cheap substring
    /// filtering. Built once alongside `display_name` and `metadata` so
    /// we only read version-info per entry once.
    searchable: Option<String>,
}

impl LauncherEntry {
    fn new(weighted: WeightedEntry) -> Self {
        Self {
            weighted,
            display_name: None,
            metadata: None,
            icon: None,
            searchable: None,
        }
    }

    /// Compute display name, metadata, and search corpus in one pass.
    /// No-op if already populated.
    fn populate(&mut self) {
        if self.searchable.is_some() {
            return;
        }

        let display = self.weighted.entry.display_name();
        let metadata = EntryMeta::from_entry(self.weighted.entry.as_ref());
        let path_stem = self
            .weighted
            .entry
            .path()
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let link_stem = self
            .weighted
            .entry
            .link_path()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mut corpus = String::new();
        for s in [
            display.as_str(),
            path_stem.as_str(),
            link_stem.as_str(),
            metadata.description.as_deref().unwrap_or(""),
            metadata.company.as_deref().unwrap_or(""),
            metadata.product_name.as_deref().unwrap_or(""),
            metadata.original_filename.as_deref().unwrap_or(""),
            metadata.file_version.as_deref().unwrap_or(""),
        ] {
            if s.is_empty() {
                continue;
            }
            if !corpus.is_empty() {
                corpus.push('\n');
            }
            corpus.push_str(s);
        }
        corpus.make_ascii_lowercase();

        self.display_name = Some(display);
        self.metadata = Some(metadata);
        self.searchable = Some(corpus);
    }

    fn display_name(&mut self) -> &str {
        self.populate();
        self.display_name.as_deref().unwrap()
    }

    fn metadata(&mut self) -> &EntryMeta {
        self.populate();
        self.metadata.as_ref().unwrap()
    }

    fn matches(&mut self, needle: &str) -> bool {
        self.populate();
        // `needle` is already lowercased by the caller; corpus is too.
        self.searchable.as_deref().unwrap().contains(needle)
    }

    fn icon(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if self.icon.is_none() {
            let entry = self.weighted.entry.as_ref();
            // Cache key only needs to be unique-per-entry within this
            // process; the link path serves when present (so two
            // shortcuts to the same target get distinct textures).
            let key = entry
                .link_path()
                .unwrap_or(entry.path())
                .to_string_lossy()
                .into_owned();
            let texture = entry
                .icon()
                .ok()
                .and_then(|fi| fi.extract_icon())
                .map(|rgba| {
                    let img = egui::ColorImage::from_rgba_unmultiplied(
                        [ICON_SIZE as usize, ICON_SIZE as usize],
                        &rgba,
                    );
                    ctx.load_texture(key, img, egui::TextureOptions::LINEAR)
                });
            self.icon = Some(texture);
        }
        self.icon.as_ref().unwrap().as_ref()
    }

    /// What we actually hand to the shell on click — the link path for
    /// shortcuts/reparse stubs, the file path for plain entries.
    fn launch_target(&self) -> &Path {
        self.weighted
            .entry
            .link_path()
            .unwrap_or(self.weighted.entry.path())
    }
}

struct Launcher {
    entries: Vec<LauncherEntry>,
    query: String,
}

impl Launcher {
    fn new(entries: Vec<LauncherEntry>) -> Self {
        Self {
            entries,
            query: String::new(),
        }
    }
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading(format!("wintheon — {} entries", self.entries.len()));
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .desired_width(f32::INFINITY)
                .hint_text("filter…"),
        );
        ui.separator();

        let needle = self.query.to_lowercase();
        let mut matches: Vec<usize> = Vec::with_capacity(self.entries.len());
        for i in 0..self.entries.len() {
            if self.entries[i].matches(&needle) {
                matches.push(i);
            }
        }

        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for idx in matches {
                render_row(&mut self.entries[idx], ui, &ctx);
            }
        });
    }
}

fn render_row(entry: &mut LauncherEntry, ui: &mut egui::Ui, ctx: &egui::Context) {
    // Snapshot every value we need before borrowing into the closure tree
    // (avoids re-entering `&mut entry` from inside the egui callbacks).
    let icon = entry.icon(ctx).cloned();
    let name = entry.display_name().to_owned();
    let meta = entry.metadata().clone();
    let origin = entry.weighted.origin.clone();
    let full_path = entry.weighted.entry.path().display().to_string();
    let launch_target = entry.launch_target().to_path_buf();

    ui.horizontal(|ui| {
        if let Some(tex) = &icon {
            ui.image((tex.id(), egui::vec2(ICON_PX, ICON_PX)));
        } else {
            ui.allocate_space(egui::vec2(ICON_PX, ICON_PX));
        }
        ui.vertical(|ui| {
            let clicked = ui
                .add(egui::Button::new(egui::RichText::new(&name).strong()))
                .clicked();

            let mut tail: Vec<String> = vec![format!("[{origin}]")];
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
            ui.label(egui::RichText::new(tail.join("  •  ")).small().weak());

            if let Some(cr) = meta.copyright.as_deref() {
                ui.label(egui::RichText::new(cr).small().weak().italics());
            }
            ui.label(
                egui::RichText::new(&full_path)
                    .small()
                    .weak()
                    .italics()
                    .monospace(),
            );

            if clicked {
                spawn(&launch_target);
            }
        });
    });
    ui.separator();
}

/// Hand the path to the shell so it routes `.lnk`, AppExec stubs, and
/// plain executables uniformly.
fn spawn(path: &Path) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .spawn();
}
