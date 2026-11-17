//! Mich Startup Master — the control center UI (egui/eframe).
//!
//! Design language: graphite control center, one amber accent, Geist for prose,
//! JetBrains Mono for machine values. Every startup route on the machine in
//! grouped rows with real schedules, one-click switches on every row, sorting
//! on every column (default: enabled first, newest change first), instant
//! cached launch, undo for every action, zero popups (in-app windows only),
//! and a resident tray. See DESIGN.md at the repo root.

use egui::Color32;
use egui::{Align2, Context, FontFamily, FontId, Id, Pos2, Rect, Sense, Stroke, Vec2};
use msm_core::model::StartupItem;
use msm_ui::view::{self, InventoryRow, SortMode};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;

use crate::tray_win::{self, TrayEvent};

pub const WINDOW_TITLE: &str = "Mich Startup Master \u{2014} Startup Control";

// ---------------------------------------------------------------------------
// Tokens — DESIGN.md. egui Color32 is RGBA from 0..=255 per channel.
// ---------------------------------------------------------------------------
const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x1A);
const SURFACE: Color32 = Color32::from_rgb(0x1D, 0x1D, 0x23);
const SURFACE_2: Color32 = Color32::from_rgb(0x26, 0x26, 0x2E);
const SURFACE_3: Color32 = Color32::from_rgb(0x2C, 0x2C, 0x35);
const BORDER: Color32 = Color32::from_rgb(0x2F, 0x2F, 0x38);
const ACCENT: Color32 = Color32::from_rgb(0xFF, 0xB4, 0x54);
const TEXT_MAIN: Color32 = Color32::from_rgb(0xED, 0xED, 0xF2);
const TEXT_MUTED: Color32 = Color32::from_rgb(0x8B, 0x8B, 0x98);
const GOOD: Color32 = Color32::from_rgb(0x3F, 0xB9, 0x50);
const DANGER: Color32 = Color32::from_rgb(0xF8, 0x51, 0x49);
const SELECTION: Color32 = Color32::from_rgb(0x33, 0x2D, 0x24);

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgba_premultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

const FAMILY_SEMI: &str = "msm-semi";
const FAMILY_MEDIUM: &str = "msm-medium";
const FAMILY_MONO_MEDIUM: &str = "msm-mono-medium";

fn f_body() -> FontFamily { FontFamily::Proportional }
fn f_medium() -> FontFamily { FontFamily::Name(std::sync::Arc::from(FAMILY_MEDIUM)) }
fn f_semi() -> FontFamily { FontFamily::Name(std::sync::Arc::from(FAMILY_SEMI)) }
fn f_mono() -> FontFamily { FontFamily::Monospace }
fn f_mono_medium() -> FontFamily { FontFamily::Name(std::sync::Arc::from(FAMILY_MONO_MEDIUM)) }

const GEIST: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
const GEIST_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Geist-Medium.ttf");
const GEIST_SEMI: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");
const JBMONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const JBMONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf");

fn install_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("geist".into(), Arc::new(egui::FontData::from_owned(GEIST.to_vec())));
    fonts.font_data.insert("geist-medium".into(), Arc::new(egui::FontData::from_owned(GEIST_MEDIUM.to_vec())));
    fonts.font_data.insert("geist-semi".into(), Arc::new(egui::FontData::from_owned(GEIST_SEMI.to_vec())));
    fonts.font_data.insert("jbmono".into(), Arc::new(egui::FontData::from_owned(JBMONO.to_vec())));
    fonts.font_data.insert("jbmono-medium".into(), Arc::new(egui::FontData::from_owned(JBMONO_MEDIUM.to_vec())));

    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "geist".into());
    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.insert(0, "jbmono".into());

    let semi_chain = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono_chain = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    fonts.families.insert(f_semi(), {
        let mut chain = vec!["geist-semi".to_string()];
        chain.extend(semi_chain.iter().filter(|k| *k != "geist-semi").cloned());
        chain
    });
    fonts.families.insert(f_medium(), {
        let mut chain = vec!["geist-medium".to_string()];
        chain.extend(semi_chain.iter().filter(|k| *k != "geist-medium").cloned());
        chain
    });
    fonts.families.insert(f_mono_medium(), {
        let mut chain = vec!["jbmono-medium".to_string()];
        chain.extend(mono_chain.iter().filter(|k| *k != "jbmono-medium").cloned());
        chain
    });
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.extreme_bg_color = SURFACE;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.window_fill = SURFACE;
    style.visuals.window_stroke = Stroke::new(1.0, BORDER);
    style.visuals.popup_shadow = Default::default();
    style.visuals.window_shadow = Default::default();
    style.visuals.selection.bg_fill = SELECTION;
    style.visuals.selection.stroke = Stroke::new(1.0, mix(BORDER, ACCENT, 0.45));
    style.visuals.widgets.inactive.bg_fill = SURFACE_2;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE_2;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.hovered.bg_fill = SURFACE_3;
    style.visuals.widgets.hovered.weak_bg_fill = SURFACE_3;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, mix(BORDER, TEXT_MAIN, 0.16));
    style.visuals.widgets.active.bg_fill = SURFACE_3;
    style.visuals.widgets.active.weak_bg_fill = SURFACE_3;
    style.visuals.widgets.open.bg_fill = SURFACE_3;
    style.visuals.widgets.noninteractive.bg_fill = SURFACE;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::new(19.0, f_semi())),
        (egui::TextStyle::Body, FontId::new(14.0, f_body())),
        (egui::TextStyle::Button, FontId::new(13.0, f_medium())),
        (egui::TextStyle::Small, FontId::new(11.5, f_medium())),
        (egui::TextStyle::Monospace, FontId::new(12.5, f_mono())),
    ]
    .into();
    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// Fonts helpers
// ---------------------------------------------------------------------------
fn elide(ui: &egui::Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let galley = ui.fonts(|f| f.layout(text.to_string(), font.clone(), color, f32::INFINITY));
    if galley.size().x <= max_width {
        return galley;
    }
    let mut low = 0usize;
    let mut high = text.len();
    while low < high {
        let mid = (low + high) / 2;
        if !text.is_char_boundary(mid) { break; }
        let candidate = format!("{}\u{2026}", &text[..mid]);
        let g = ui.fonts(|f| f.layout(candidate.clone(), font.clone(), color, f32::INFINITY));
        if g.size().x <= max_width { low = mid + 1; } else { high = mid; }
    }
    let mut end = low.saturating_sub(1);
    while end > 0 && !text.is_char_boundary(end) { end -= 1; }
    ui.fonts(|f| f.layout(format!("{}\u{2026}", &text[..end]), font, color, f32::INFINITY))
}

fn measure(ui: &egui::Ui, text: &str, font: FontId) -> f32 {
    ui.fonts(|f| f.layout(text.to_string(), font, Color32::WHITE, f32::INFINITY)).size().x
}

// ---------------------------------------------------------------------------
// Worker plumbing
// ---------------------------------------------------------------------------
struct ScanOutcome {
    items: Vec<StartupItem>,
    fresh: Option<String>,
}

struct MutationOutcome {
    result: Result<(String, Option<UndoAction>), String>,
    note_routes: Option<Vec<StartupItem>>,
}

#[derive(Clone)]
enum UndoAction {
    Toggle { routes: Vec<StartupItem>, now_enabled: bool },
    AddManaged { task: String },
    DeleteManaged { location: String, xml: String },
    Purge { backup_id: String, label: String },
    EditRollback { item: StartupItem, name: String, target: String, args: String, quiet: bool },
    Hidden { keys: Vec<String>, label: String },
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------
#[derive(Default)]
struct AddDialog {
    name: String,
    program: String,
    arguments: String,
    quiet: bool,
}

struct EditDialog {
    item: StartupItem,
    name: String,
    program: String,
    arguments: String,
    quiet: bool,
    can_mode: bool,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------
pub struct MsmApp {
    items: Vec<StartupItem>,
    rows: Vec<InventoryRow>,
    filter: &'static str,
    query: String,
    sort: SortMode,
    selected_key: Option<String>,
    expanded: HashSet<String>,
    scanning: Arc<AtomicBool>,
    status: String,
    status_is_error: bool,
    hidden_count: usize,
    undo: Option<(String, UndoAction, f64)>,
    scan_tx: Sender<ScanOutcome>,
    scan_rx: Receiver<ScanOutcome>,
    mutation_tx: Sender<MutationOutcome>,
    mutation_rx: Receiver<MutationOutcome>,
    tray_rx: Receiver<TrayEvent>,
    add_dialog: Option<AddDialog>,
    edit_dialog: Option<EditDialog>,
    focus_search: bool,
    quitting: bool,
    hidden_note_shown: bool,
}

pub fn run(start_in_tray: bool) -> i32 {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size(Vec2::new(1380.0, 880.0))
            .with_min_inner_size(Vec2::new(940.0, 660.0))
            .with_visible(!start_in_tray),
        ..Default::default()
    };
    match eframe::run_native("MichStartupMaster", options, Box::new(|cc| {
        install_fonts(&cc.egui_ctx);
        Ok(Box::new(MsmApp::new(&cc.egui_ctx)))
    })) {
        Ok(()) => 0,
        Err(e) => { eprintln!("ui error: {}", e); 1 }
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

impl MsmApp {
    pub fn new(ctx: &Context) -> MsmApp {
        let (scan_tx, scan_rx) = std::sync::mpsc::channel::<ScanOutcome>();
        let (mutation_tx, mutation_rx) = std::sync::mpsc::channel::<MutationOutcome>();
        let tray_rx = tray_win::spawn(ctx.clone());
        // Instant first paint: the last complete inventory renders immediately;
        // a live scan replaces it a moment later.
        let cached = msm_state::cache::load().unwrap_or_default();
        let mut app = MsmApp {
            rows: Vec::new(),
            items: cached,
            filter: "Apps",
            query: String::new(),
            sort: SortMode::Smart,
            selected_key: None,
            expanded: HashSet::new(),
            scanning: Arc::new(AtomicBool::new(false)),
            status: "Scanning every startup surface\u{2026}".into(),
            status_is_error: false,
            hidden_count: msm_state::denied::list().len(),
            undo: None,
            scan_tx,
            scan_rx,
            mutation_tx,
            mutation_rx,
            tray_rx,
            add_dialog: None,
            edit_dialog: None,
            focus_search: false,
            quitting: false,
            hidden_note_shown: false,
        };
        app.rebuild_rows();
        app.kick_scan(ctx);
        app.start_change_monitor(ctx);
        app
    }

    fn rebuild_rows(&mut self) {
        self.rows = view::build_visible_rows_sorted(&self.items, self.filter, &self.query, self.sort);
        let base = self.items.iter().map(|x| msm_state::first_seen::utc_of(x)).filter(|v| *v > 0).min().unwrap_or(0);
        BASELINE_NS.store(base as u64, std::sync::atomic::Ordering::Relaxed);
        if let Some(key) = &self.selected_key {
            if !self.rows.iter().any(|r| &r.key == key) {
                self.selected_key = None;
            }
        }
        // Something is always selected: the selection language (tint + accent
        // bar) is visible from the first frame, and Enter always acts.
        if self.selected_key.is_none() {
            if let Some(first) = self.rows.first() {
                self.selected_key = Some(first.key.clone());
            }
        }
        self.hidden_count = msm_state::denied::list().len();
    }

    fn kick_scan(&self, ctx: &Context) {
        // Watchdog: a wedged scan (dead drive, stuck WMI) must never freeze
        // refreshes forever — after 45s a new scan takes over the flag.
        static LAST_KICK_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_ms = msm_core::util::tick_ms() as u64;
        if self.scanning.swap(true, Ordering::SeqCst) {
            let last = LAST_KICK_MS.load(Ordering::SeqCst);
            if now_ms.saturating_sub(last) < 45_000 {
                return;
            }
        }
        LAST_KICK_MS.store(now_ms, Ordering::SeqCst);
        let tx = self.scan_tx.clone();
        let scanning = self.scanning.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new().name("inventory-refresh".into()).spawn(move || {
            // Bounded: a hung provider surfaces as a skipped cycle, never a dead UI.
            let scanned = msm_core::scan::scan_all_within(25_000);
            let items = scanned.unwrap_or_default();
            if items.is_empty() {
                scanning.store(false, Ordering::SeqCst);
                return;
            }
            msm_state::first_seen::observe(&items);
            let fresh = msm_state::known::detect_new_with(&items);
            let fresh_name = fresh.first().map(|f| f.human_name().to_string());
            scanning.store(false, Ordering::SeqCst);
            let _ = tx.send(ScanOutcome { items, fresh: fresh_name });
            ctx.request_repaint();
        }).ok();
    }

    fn start_change_monitor(&self, ctx: &Context) {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let tx = self.scan_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new().name("change-monitor".into()).spawn(move || {
                let monitor = Arc::new(msm_watch::StartupChangeMonitor::new(move || {
                    // Registry/folder/task-catalog change → rescan through the shared
                    // guarded channel (scan_tx receives only full ScanOutcomes, so a
                    // rescan here funnels through kick logic on the UI side; the
                    // monitor instead refreshes by direct scan to keep this simple).
                    let items = msm_core::scan::scan_all();
                    msm_state::first_seen::observe(&items);
                    let _ = tx.send(ScanOutcome { items, fresh: None });
                    ctx.request_repaint();
                }));
                monitor.start();
                loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
            }).ok();
        });
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected_key.as_ref().and_then(|key| self.rows.iter().position(|r| &r.key == key))
    }

    fn set_status(&mut self, text: String, is_error: bool) {
        self.status = text;
        self.status_is_error = is_error;
    }

    /// Run a slow mutation on a worker thread; the UI never blocks. The job
    /// returns its success message and (when it can) the undo action.
    fn run_job(&mut self, ctx: &Context, working: impl Into<String>, note_routes: Option<Vec<StartupItem>>,
               job: impl FnOnce() -> Result<(String, Option<UndoAction>), String> + Send + 'static) {
        self.set_status(working.into(), false);
        let tx = self.mutation_tx.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new().name("msm-mutation".into()).spawn(move || {
            let result = job();
            let _ = tx.send(MutationOutcome { result, note_routes });
            ctx.request_repaint();
        }).ok();
    }

    fn toggle_row(&mut self, ctx: &Context, index: usize) {
        let Some(row) = self.rows.get(index).cloned() else { return };
        self.selected_key = Some(row.key.clone());
        let enable_requested = !row.effective_enabled();
        let candidates = view::bulk_state_candidates(&row, enable_requested);
        if candidates.is_empty() {
            self.set_status(format!("{} \u{2014} every entry is already {}.", row.display_name, if enable_requested { "on" } else { "off" }), false);
            return;
        }
        let label = row.display_name.clone();
        let count = candidates.len();
        let undo_routes = candidates.clone();
        self.run_job(ctx,
            format!("{} {} startup {} for {}.", if enable_requested { "Enabling" } else { "Disabling" },
                count, if count == 1 { "entry" } else { "entries" }, label),
            Some(undo_routes),
            move || {
                // Best effort per route: one stubborn route never leaves the rest half done (no Mixed).
                let (done, failures) = msm_actions::set_routes_best_effort(&candidates, enable_requested);
                let result: Result<usize, String> = if done == 0 && !failures.is_empty() { Err(failures.join(" | ")) } else { Ok(done) };
                result.map(|applied| (format!("{} {} startup {} for {}.", if enable_requested { "Enabled" } else { "Disabled" }, applied,
                    if applied == 1 { "entry" } else { "entries" }, label),
                    Some(UndoAction::Toggle { routes: candidates.clone(), now_enabled: enable_requested })))
            });
    }

    fn toggle_route(&mut self, ctx: &Context, route: StartupItem) {
        let enable_requested = !route.enabled;
        let name = route.human_name().to_string();
        self.run_job(ctx,
            format!("{} {}.", if enable_requested { "Enabling" } else { "Disabling" }, name),
            Some(vec![route.clone()]),
            move || {
                if enable_requested { msm_actions::enable_forced(&route) } else { msm_actions::disable_forced(&route) }
                    .map(|_| (format!("{} {}.", if enable_requested { "Enabled" } else { "Disabled" }, name),
                        Some(UndoAction::Toggle { routes: vec![route.clone()], now_enabled: enable_requested })))
            });
    }

    fn purge_row(&mut self, ctx: &Context, row: &InventoryRow) {
        let routes = row.routes.clone();
        let label = row.display_name.clone();
        self.run_job(ctx, format!("Purging {} everywhere\u{2026}", label), None, move || {
            msm_actions::purge::purge_all(&routes, &label).map(|receipt| {
                let message = format!("Purged {} everywhere \u{00B7} {} removed \u{00B7} restore it from the Hidden view.", label, receipt.removed);
                let undo = if receipt.backup_id.is_empty() { None } else {
                    Some(UndoAction::Purge { backup_id: receipt.backup_id, label: label.clone() })
                };
                (message, undo)
            })
        });
    }

    fn copy_all(&mut self, ctx: &Context) {
        let items = self.items.clone();
        if items.is_empty() {
            self.set_status("Nothing to copy yet.".into(), false);
            return;
        }
        let tsv = view::build_startup_export_tsv(&items);
        ctx.output_mut(|o| o.copied_text = tsv);
        self.set_status(format!("Copied all {} startup entries with every detail.", items.len()), false);
    }

    fn copy_selected(&mut self, ctx: &Context, row: &InventoryRow) {
        let tsv = view::build_startup_export_tsv(&row.routes);
        ctx.output_mut(|o| o.copied_text = tsv);
        self.set_status(format!("Copied {} for {} with every detail.", if row.routes.len() == 1 { "the startup entry" } else { "all routes" }, row.display_name), false);
    }

    fn paste_and_add(&mut self, ctx: &Context) {
        let clipboard = read_clipboard_text();
        if clipboard.trim().is_empty() {
            self.set_status("The clipboard is empty \u{2014} copy a full program path first.".into(), true);
            return;
        }
        self.add_from_path(ctx, clipboard);
    }

    fn add_from_path(&mut self, ctx: &Context, pasted: String) {
        self.run_job(ctx, "Adding from the pasted path\u{2026}", None, move || {
            msm_actions::managed::add_startup_from_paste(&pasted).map(|(task, name)|
                (format!("Added {} \u{00B7} runs once at every sign-in.", name),
                 Some(UndoAction::AddManaged { task })))
        });
    }

    fn run_now(&mut self, ctx: &Context, item: &StartupItem) {
        let target_item = item.clone();
        let name = item.human_name().to_string();
        self.run_job(ctx, format!("Starting {}.", name), None, move || {
            msm_actions::resolve_launch_target_or_fail(&target_item).map(|(target, arguments)| {
                use std::os::windows::process::CommandExt;
                let working_directory = target.rsplit_once('\\').map(|(d, _)| d.to_string()).unwrap_or_default();
                let _ = std::process::Command::new(&target).raw_arg(&arguments)
                    .current_dir(working_directory).creation_flags(0x0800_0000).spawn();
                (format!("Started {}.", name), None)
            })
        });
    }

    fn open_location(&mut self, _ctx: &Context, item: &StartupItem) {
        let target = msm_actions::resolve_launch_target_or_fail(item).map(|(t, _)| t).unwrap_or_default();
        let path = std::path::PathBuf::from(msm_core::util::expand_env(target.trim().trim_matches('"')));
        let pick = if path.is_file() { Some(path) } else { path.parent().filter(|p| p.exists()).map(|p| p.to_path_buf()) };
        if let Some(location) = pick {
            let _ = std::process::Command::new("explorer.exe").arg(format!("/select,\"{}\"", location.display())).spawn();
            self.set_status(format!("Opened the location of {}.", item.human_name()), false);
        } else if item.location.starts_with("HK") {
            let _ = std::process::Command::new("regedit.exe").spawn();
            self.set_status(format!("Opened Registry Editor for {}.", item.human_name()), false);
        } else {
            self.set_status("No on-disk location for this entry.".into(), false);
        }
    }

    fn delete_managed(&mut self, ctx: &Context, item: &StartupItem) {
        let location = item.location.clone();
        let name = item.human_name().to_string();
        let xml = msm_core::tasks_com::try_read_task_xml(&location).unwrap_or_default();
        self.run_job(ctx, format!("Deleting the managed task for {}.", name), None, move || {
            msm_actions::managed::delete_managed_task(&location).map(|_|
                (format!("Deleted the managed task for {}.", name),
                 Some(UndoAction::DeleteManaged { location: location.clone(), xml: xml.clone() })))
        });
    }

    fn set_managed_mode(&mut self, ctx: &Context, item: &StartupItem, quiet: bool) {
        let (target, arguments) = msm_actions::resolve_launch_target_or_fail(item)
            .unwrap_or_else(|_| split_command(&item.command));
        let name = item.human_name().to_string();
        let owned = item.clone();
        let old_quiet = msm_core::quiet_plan::is_quiet_launch(&item.command, &item.location);
        self.run_job(ctx, format!("Switching {} to {}.", name, if quiet { "quiet (tray)" } else { "window" }), None, move || {
            msm_actions::managed::edit_startup(&owned, &name, &target, &arguments, quiet)
                .map(|_| (format!("{} now starts {}.", name, if quiet { "quietly in the tray" } else { "in a window" }),
                    Some(UndoAction::EditRollback { item: owned.clone(), name: name.clone(), target: target.clone(), args: arguments.clone(), quiet: old_quiet })))
        });
    }

    fn hide_row(&mut self, ctx: &Context, row: &InventoryRow) {
        let routes = row.routes.clone();
        let label = row.display_name.clone();
        let keys = msm_actions::purge::hide(&routes, &label);
        let now = ctx.input(|i| i.time);
        self.set_status(format!("Hid {} \u{2014} restore it from the Hidden view any time.", label), false);
        self.undo = Some((format!("Hid {}.", label), UndoAction::Hidden { keys, label }, now + 15.0));
        self.rebuild_rows();
    }
}

fn split_command(command: &str) -> (String, String) {
    let trimmed = command.trim();
    if let Some(stripped) = trimmed.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            return (stripped[..end].to_string(), stripped[end + 1..].trim().to_string());
        }
    }
    match trimmed.split_once(' ') {
        Some((head, rest)) => (head.to_string(), rest.trim().to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

/// egui 0.32 exposes no clipboard READ API — read CF_UNICODETEXT directly.
fn read_clipboard_text() -> String {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::GlobalLock;
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        if !OpenClipboard(None).is_ok() { return String::new(); }
        let mut text = String::new();
        if let Ok(handle) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(handle.0)) as *const u16;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 1_048_576 { len += 1; }
                text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                let _ = windows::Win32::System::Memory::GlobalUnlock(windows::Win32::Foundation::HGLOBAL(handle.0));
            }
        }
        let _ = CloseClipboard();
        text
    }
}

// ---------------------------------------------------------------------------
// Layout metrics (px; egui is dpi-scaled automatically by eframe)
// ---------------------------------------------------------------------------
struct Cols {
    width: f32,
    margin: f32,
    name_x: f32,
    schedule_x: f32,
    source_x: f32,
    toggle_x: f32,
    toggle_w: f32,
    chip_max: f32,
    row_h: f32,
    sub_h: f32,
}

fn cols(width: f32) -> Cols {
    let margin = 24.0;
    let toggle_w = 38.0;
    let toggle_x = width - margin - toggle_w;
    let schedule_x = (width * 0.44).clamp(400.0, 640.0);
    Cols {
        width,
        margin,
        name_x: margin + 30.0,
        schedule_x,
        source_x: schedule_x + 234.0,
        toggle_x,
        toggle_w,
        chip_max: 218.0,
        row_h: 58.0,
        sub_h: 44.0,
    }
}

impl eframe::App for MsmApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        // Tray events.
        let quitting = tray_win::poll(&self.tray_rx, |event| match event {
            TrayEvent::Open => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            TrayEvent::Refresh => self.kick_scan(ctx),
            TrayEvent::Quit => {}
        });
        if quitting {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // Scan outcomes.
        loop {
            match self.scan_rx.try_recv() {
                Ok(outcome) => {
                    self.items = outcome.items;
                    msm_state::cache::save(&self.items);
                    self.rebuild_rows();
                    let enabled = self.items.iter().filter(|x| x.enabled).count();
                    if let Some(name) = outcome.fresh {
                        self.set_status(format!("New startup entry detected: {} \u{2014} it is in the list now.", name), false);
                    } else {
                        self.set_status(format!("{} startup entries \u{00B7} {} on \u{00B7} {} off \u{00B7} live monitoring on", self.items.len(), enabled, self.items.len() - enabled), false);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        // Mutation outcomes.
        loop {
            match self.mutation_rx.try_recv() {
                Ok(outcome) => {
                    if let Some(routes) = &outcome.note_routes {
                        if outcome.result.is_ok() {
                            view::note_change(routes);
                        }
                    }
                    match outcome.result {
                        Ok((message, undo)) => {
                            self.set_status(message.clone(), false);
                            if let Some(undo) = undo {
                                self.undo = Some((message, undo, now + 15.0));
                            }
                        }
                        Err(error) => self.set_status(format!("Action failed \u{2014} everything was left unchanged. {}", error), true),
                    }
                    self.kick_scan(ctx);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        // Undo expiry.
        if let Some((_, _, deadline)) = self.undo {
            if now > deadline {
                self.undo = None;
            }
        }
        // Global keyboard.
        self.global_keys(ctx);
        // Close → tray (cancel the exit, hide the window).
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if !self.hidden_note_shown {
                self.set_status("Hidden to the tray \u{2014} the monitor keeps watching. Quit lives on the tray menu.".into(), false);
                self.hidden_note_shown = true;
            }
        }

        self.toolbar(ctx, now);
        self.sort_header(ctx);
        self.status_bar(ctx, now);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(16, 4)))
            .show(ctx, |ui| {
                if self.filter == "Hidden" {
                    self.hidden_view(ui, ctx);
                } else {
                    self.list_view(ui, ctx);
                }
            });

        if self.scanning.load(Ordering::SeqCst) {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
        if self.undo.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
        self.dialogs(ctx);
    }
}

/// Earliest first-seen stamp (ns): rows first seen later than this + 60 s are "new".
static PURGE_ARMED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static BASELINE_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn is_new_row(row: &InventoryRow) -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let base = BASELINE_NS.load(std::sync::atomic::Ordering::Relaxed) as u128;
    let seen = row.routes.iter().map(msm_state::first_seen::utc_of).max().unwrap_or(0);
    seen > 0 && seen > base + 60_000_000_000 && now.saturating_sub(seen) < 86_400_000_000_000
}

impl MsmApp {
    fn global_keys(&mut self, ctx: &Context) {
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        let text_focused = ctx.memory(|m| m.has_focus(Id::new("msm-search")));
        if ctrl && ctx.input(|i| i.key_pressed(egui::Key::F)) {
            self.focus_search = true;
        }
        if ctrl && ctx.input(|i| i.key_pressed(egui::Key::C)) && !text_focused {
            self.copy_all(ctx);
        }
        if ctrl && ctx.input(|i| i.key_pressed(egui::Key::V)) && !text_focused {
            self.paste_and_add(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) || (ctrl && ctx.input(|i| i.key_pressed(egui::Key::R))) {
            self.kick_scan(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) && !text_focused {
            if let Some(index) = self.selected_index() {
                self.toggle_row(ctx, index);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && !text_focused {
            if let Some(index) = self.selected_index() {
                self.toggle_row(ctx, index);
            }
        }
        let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
        let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
        if (down || up) && !self.rows.is_empty() {
            let current = self.selected_index().unwrap_or(0);
            let next = if down { (current + 1).min(self.rows.len() - 1) } else { current.saturating_sub(1) };
            self.selected_key = Some(self.rows[next].key.clone());
            ctx.request_repaint();
        }
    }

    fn toolbar(&mut self, ctx: &Context, _now: f64) {
        egui::TopBottomPanel::top("toolbar").frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(16, 10))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 10.0;
                // Brand.
                let (brand_rect, _) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), Sense::hover());
                let center = brand_rect.center();
                ui.painter().rect_filled(brand_rect, 4.0, ACCENT);
                let _ = center;
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Mich Startup Master").family(f_semi()).size(15.0).color(TEXT_MAIN));
                ui.add_space(16.0);
                let (divider, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                ui.painter().rect_filled(divider, 0.0, BORDER);
                ui.add_space(16.0);
                // Filters.
                let enabled_count = self.items.iter().filter(|x| x.enabled).count();
                let disabled_count = self.items.len() - enabled_count;
                for (label, tag, count) in [
                    ("All", "Apps", self.items.len()),
                    ("On", "Enabled", enabled_count),
                    ("Off", "Disabled", disabled_count),
                    ("Hidden", "Hidden", self.hidden_count),
                ] {
                    let active = self.filter == tag;
                    let text = egui::RichText::new(format!("{} {}", label, count))
                        .family(if active { f_semi() } else { f_medium() })
                        .size(12.5)
                        .color(if active { ACCENT } else { TEXT_MUTED });
                    if ui.add(egui::Button::new(text).fill(Color32::TRANSPARENT).stroke(egui::Stroke::NONE)).clicked() {
                        self.filter = tag;
                        self.selected_key = None;
                        self.rebuild_rows();
                        self.set_status(format!("{} view \u{00B7} {} rows", label, self.rows.len()), false);
                    }
                }
                ui.add_space(8.0);
                // Search (fixed slot so the toolbar rhythm never jumps).
                let mut search = egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search  (Ctrl+F)")
                    .desired_width(280.0)
                    .font(egui::TextStyle::Body);
                if self.focus_search {
                    search = search.id(Id::new("msm-search"));
                }
                let response = ui.add(search);
                if self.focus_search {
                    response.request_focus();
                    self.focus_search = false;
                }
                if response.changed() {
                    self.rebuild_rows();
                }
                // Actions.
                if ui.add(egui::Button::new(egui::RichText::new("Copy all").family(f_medium()).size(12.5).color(TEXT_MAIN))
                    .fill(SURFACE_2).stroke(Stroke::new(1.0, BORDER))).on_hover_text("Copy every startup entry with all details (Ctrl+C)").clicked() {
                    self.copy_all(ctx);
                }
                if ui.add(egui::Button::new(egui::RichText::new("Paste & add").family(f_medium()).size(12.5).color(TEXT_MAIN))
                    .fill(SURFACE_2).stroke(Stroke::new(1.0, BORDER))).on_hover_text("Copy a program path anywhere, then click here (Ctrl+V) \u{2014} the name and task are automatic").clicked() {
                    self.paste_and_add(ctx);
                }
                if ui.add(egui::Button::new(egui::RichText::new("+ Add").family(f_medium()).size(12.5).color(TEXT_MAIN))
                    .fill(SURFACE_2).stroke(Stroke::new(1.0, BORDER))).clicked() {
                    self.add_dialog = Some(AddDialog::default());
                }
                let (divider2, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                ui.painter().rect_filled(divider2, 0.0, BORDER);
                self.tools_menu(ui, ctx);
                // Primary verb on selection — danger-ghost styling: a bulk
                // enable/disable is not the brand CTA; it must read as an action,
                // colored by intent.
                let verb_info = self.selected_index()
                    .and_then(|i| self.rows.get(i).map(|row| {
                        let enable = !row.effective_enabled();
                        let count = view::bulk_state_candidates(row, enable).len().max(1);
                        (format!("{} \u{00B7} {}", if enable { "Enable" } else { "Disable" }, count), enable)
                    }));
                let (verb, enable_intent) = verb_info.unwrap_or_else(|| ("Disable \u{00B7} 1".into(), false));
                let verb_color = if enable_intent { GOOD } else { DANGER };
                if ui.add(egui::Button::new(egui::RichText::new(verb).family(f_semi()).size(12.5).color(verb_color))
                    .fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, verb_color))).clicked() {
                    if let Some(index) = self.selected_index() {
                        self.toggle_row(ctx, index);
                    }
                }
            });
        });
    }

    fn tools_menu(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        egui::menu::menu_button(ui, "Tools", |ui| {
            if ui.button("Refresh now").clicked() {
                self.kick_scan(ctx);
                ui.close_menu();
            }
            let selection = self.selected_index().and_then(|i| self.rows.get(i).cloned());
            if ui.button("Protect disabled entries").clicked() {
                let targets = selection.as_ref().map(|row| row.routes.clone()).unwrap_or_else(|| self.items.iter().filter(|x| !x.enabled).cloned().collect());
                let mut count = 0usize;
                for item in &targets {
                    if !item.enabled {
                        msm_state::protected_disabled::protect(item);
                        count += 1;
                    }
                }
                self.set_status(format!("Protected {} disabled entries \u{2014} repair will never switch them back on.", count), false);
                ui.close_menu();
            }
            if ui.button("Repair startup rules").clicked() {
                let ctx2 = ctx.clone();
                self.run_job(&ctx2, "Repairing startup rules\u{2026}", None, move || {
                    msm_actions::reconcile::reconcile_managed_startup_routes(None)
                        .map(|report| (format!("Repair finished: {}", report), None))
                });
                ui.close_menu();
            }
            if ui.button("Verify coverage").clicked() {
                let (passed, receipt) = msm_ui::verify::verify_live_inventory();
                self.set_status(format!("Coverage {}: {}", if passed { "verified" } else { "needs attention" }, receipt), !passed);
                ui.close_menu();
            }
            if ui.button("Open startup folders").clicked() {
                let _ = std::process::Command::new("explorer.exe").arg("shell:startup").spawn();
                let _ = std::process::Command::new("explorer.exe").arg("shell:common startup").spawn();
                self.set_status("Opened the personal and shared startup folders.".into(), false);
                ui.close_menu();
            }
            if ui.button("Show hidden entries").clicked() {
                self.filter = "Hidden";
                self.rebuild_rows();
                ui.close_menu();
            }
            if let Some(row) = selection.as_ref().filter(|row| row.has_managed()) {
                ui.separator();
                let item = row.primary_item().clone();
                if ui.button("Quiet (tray) mode").clicked() {
                    self.set_managed_mode(ctx, &item, true);
                    ui.close_menu();
                }
                if ui.button("Window mode").clicked() {
                    self.set_managed_mode(ctx, &item, false);
                    ui.close_menu();
                }
            }
        });
    }

    fn sort_header(&mut self, ctx: &Context) {
        let sort = self.sort;
        egui::TopBottomPanel::top("sort-header").frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(16, 6))).show(ctx, |ui| {
            let width = ui.available_width();
            let c = cols(width);
            let full = ui.available_rect_before_wrap();
            let mut draw = |ui: &mut egui::Ui, label: &str, mode: SortMode, x: f32, right: bool| {
                let active = sort == mode;
                let text = if active { format!("{} \u{25BE}", label) } else { label.to_string() };
                let font = FontId::new(11.5, f_medium());
                let tw = measure(ui, &text, font.clone());
                let hit_w = tw + 16.0;
                let left_x = full.min.x + if right { x - hit_w } else { x };
                let rect = Rect::from_min_size(Pos2::new(left_x, full.min.y), Vec2::new(hit_w, 18.0));
                let response = ui.interact(rect, Id::new(("col", label)), Sense::click());
                ui.painter().text(rect.min, Align2::LEFT_TOP, text, font, if active { ACCENT } else { TEXT_MUTED });
                if response.clicked() {
                    self.sort = if active { SortMode::Smart } else { mode };
                    self.rebuild_rows();
                }
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            };
            draw(ui, "Startup item", SortMode::Name, c.name_x - 6.0, false);
            draw(ui, "Status", SortMode::Status, c.schedule_x - 20.0, true);
            draw(ui, "Schedule", SortMode::Schedule, c.schedule_x, false);
            draw(ui, "Source", SortMode::Source, c.source_x, false);
            draw(ui, "Impact", SortMode::Impact, c.toggle_x - 26.0, true);
            ui.add_space(18.0);
        });
    }

    fn list_view(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let width = ui.available_width();
        if self.rows.is_empty() {
            let mid = ui.available_rect_before_wrap();
            let message = if self.scanning.load(Ordering::SeqCst) && self.items.is_empty() {
                "Scanning every startup surface\u{2026}"
            } else if !self.query.trim().is_empty() {
                "Nothing matches this search."
            } else {
                "No startup entries in this view."
            };
            ui.painter().text(mid.center(), Align2::CENTER_CENTER, message, FontId::new(15.0, f_medium()), TEXT_MUTED);
            return;
        }
        let c = cols(width);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            let mut index = 0usize;
            while index < self.rows.len() {
                let row = self.rows[index].clone();
                self.group_row(ui, ctx, &row, index, &c);
                index += 1;
            }
            ui.add_space(8.0);
        });
    }

    fn group_row(&mut self, ui: &mut egui::Ui, ctx: &Context, row: &InventoryRow, index: usize, c: &Cols) {
        let expanded = self.expanded.contains(&row.key) && row.routes.len() > 1;
        let group_h = c.row_h + if expanded { c.sub_h * row.routes.len() as f32 } else { 0.0 };
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, group_h), Sense::click());
        let painter = ui.painter().with_clip_rect(rect);
        let selected = self.selected_key.as_deref() == Some(row.key.as_str());

        // Group face.
        let face = if selected { SELECTION }
            else if response.hovered() { mix(SURFACE, TEXT_MAIN, 0.045) }
            else if index % 2 == 1 { mix(BG, SURFACE, 0.72) }
            else { BG };
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(width, c.row_h)), 7.0, face);
        if selected {
            painter.rect_filled(Rect::from_min_max(
                Pos2::new(rect.min.x + 2.0, rect.min.y + 8.0),
                Pos2::new(rect.min.x + 5.0, rect.min.y + c.row_h - 8.0)), 2.0, ACCENT);
        }

        let center_y = rect.min.y + c.row_h / 2.0;
        let all_enabled = row.all_enabled();
        let all_disabled = row.all_disabled();

        // Recent-change halo around the dot.
        let changed = row.changed_at();
        let recent = changed > 0 && now_nanos().saturating_sub(changed) < 12_000_000_000;
        let dot_color = if all_enabled { GOOD } else if all_disabled { DANGER } else { ACCENT };
        let dot_x = rect.min.x + c.margin + 9.0;
        if recent {
            painter.circle_filled(Pos2::new(dot_x, center_y), 11.0, dot_color.gamma_multiply(0.16));
        }
        let dot_pos = Pos2::new(dot_x, center_y);
        painter.circle_filled(dot_pos, 6.0, dot_color);
        painter.circle_stroke(dot_pos, 6.0, Stroke::new(1.0, mix(BG, dot_color, 0.45)));

        // Name + entry (two calm lines), clipped to the schedule column.
        let text_zone = Rect::from_min_max(
            Pos2::new(rect.min.x + c.name_x, rect.min.y + 9.0),
            Pos2::new(rect.min.x + c.schedule_x - 110.0, rect.min.y + c.row_h));
        let clip = ui.painter().with_clip_rect(text_zone);
        let name_galley = elide(ui, &row.display_name, FontId::new(14.5, f_semi()), TEXT_MAIN, text_zone.width());
        let name_w = name_galley.size().x;
        clip.galley(Pos2::new(text_zone.min.x, text_zone.min.y), name_galley, TEXT_MAIN);
        if is_new_row(row) && name_w + 44.0 < text_zone.width() {
            let b = Rect::from_min_size(Pos2::new(text_zone.min.x + name_w + 8.0, text_zone.min.y + 2.0), Vec2::new(34.0, 17.0));
            painter.rect_filled(b, 5.0, mix(SURFACE_2, ACCENT, 0.35));
            painter.text(b.center(), Align2::CENTER_CENTER, "NEW", FontId::new(10.5, f_semi()), TEXT_MAIN);
        }
        let entry_text = entry_hint(row.primary_item());
        let entry_galley = elide(ui, &entry_text, FontId::new(12.0, f_mono()), TEXT_MUTED, text_zone.width());
        clip.galley(Pos2::new(text_zone.min.x, text_zone.min.y + 26.0), entry_galley, TEXT_MUTED);

        // Expansion chevron for grouped apps.
        if row.routes.len() > 1 {
            let chevron = if expanded { "\u{25BE}" } else { "\u{25B8}" };
            painter.text(Pos2::new(rect.min.x + c.name_x - 22.0, center_y - 7.0), Align2::LEFT_TOP,
                chevron, FontId::new(12.0, f_medium()), TEXT_MUTED);
        }

        // Status word — one calm encoding: On / Off / Mixed (+ route count).
        let status_label = if row.routes.len() > 1 {
            format!("{} \u{00B7} {}", if all_enabled { "On" } else if all_disabled { "Off" } else { "Mixed" }, row.routes.len())
        } else if all_enabled { "On".into() } else if all_disabled { "Off".into() } else { "Mixed".into() };
        let status_color = if all_enabled { GOOD } else if all_disabled { DANGER } else { ACCENT };
        let status_font = FontId::new(11.5, f_semi());
        let sw = measure(ui, &status_label, status_font.clone());
        painter.text(Pos2::new(rect.min.x + c.schedule_x - 20.0 - sw, center_y - 8.0), Align2::LEFT_TOP, &status_label, status_font, status_color);

        // Schedule chip — uniform min width so the column holds a rhythm.
        let schedule = row.schedule_text();
        if !schedule.is_empty() {
            let chip_font = FontId::new(12.0, f_mono_medium());
            let tw = measure(ui, &schedule, chip_font.clone());
            let chip_w = (tw + 20.0).clamp(104.0, c.chip_max);
            let chip_h = 24.0;
            let chip = Rect::from_min_size(Pos2::new(rect.min.x + c.schedule_x, center_y - chip_h / 2.0), Vec2::new(chip_w, chip_h));
            painter.rect_filled(chip, 7.0, mix(SURFACE_2, ACCENT, 0.13));
            painter.rect_stroke(chip, 7.0, Stroke::new(1.0, mix(BORDER, ACCENT, 0.35)), egui::StrokeKind::Middle);
            let chip_clip = ui.painter().with_clip_rect(chip);
            let galley = elide(ui, &schedule, chip_font, ACCENT, chip_w - 14.0);
            chip_clip.galley(Pos2::new(chip.min.x + 9.0, chip.center().y - galley.size().y / 2.0), galley, ACCENT);
        }

        // Source (clipped so wide chips never collide with it).
        let source_zone = Rect::from_min_max(
            Pos2::new(rect.min.x + c.source_x, center_y - 8.0),
            Pos2::new(rect.min.x + c.toggle_x - 86.0, center_y + 10.0));
        let source_clip = ui.painter().with_clip_rect(source_zone.expand(40.0));
        let source_galley = elide(ui, &short_source(&row.source_text()), FontId::new(11.5, f_medium()), TEXT_MUTED, source_zone.width());
        source_clip.galley(source_zone.min, source_galley, TEXT_MUTED);

        // Impact, right-aligned before the toggle (aligned with its header).
        let impact = row.aggregate_impact_text();
        let impact_color = match impact.as_str() { "High impact" => DANGER, "Medium impact" => ACCENT, _ => TEXT_MUTED };
        let iw = measure(ui, &impact, FontId::new(11.5, f_medium()));
        painter.text(Pos2::new(rect.min.x + c.toggle_x - 26.0 - iw, center_y - 7.0), Align2::LEFT_TOP,
            &impact, FontId::new(11.5, f_medium()), impact_color);

        // Toggle.
        let track = Rect::from_min_size(Pos2::new(rect.min.x + c.toggle_x, center_y - 10.5), Vec2::new(c.toggle_w, 21.0));
        let enabled = row.effective_enabled();
        let target = if enabled { 1.0 } else { 0.0 };
        let t = ui.ctx().animate_value_with_time(Id::new(row.key.clone()).with("tg"), target, 0.16);
        draw_switch(&painter, track, t, ui.ctx().animate_value_with_time(Id::new(row.key.clone()).with("hover"), if response.hovered() { 1.0 } else { 0.0 }, 0.1));

        // Route count badge / expand affordance.
        if row.routes.len() > 1 {
            let badge = format!("{} \u{00B7} open the arrow to see each one", row.routes.len());
            let _ = badge;
        }

        // Sub-rows.
        if expanded {
            let sub_top = rect.min.y + c.row_h;
            for (i, route) in row.routes.iter().enumerate() {
                let sub = Rect::from_min_size(Pos2::new(rect.min.x + c.margin, sub_top + c.sub_h * i as f32), Vec2::new(width - c.margin * 2.0, c.sub_h - 4.0));
                let sub_painter = ui.painter().with_clip_rect(sub);
                let route_enabled = view::display_state(route) == "Enabled";
                sub_painter.rect_filled(Rect::from_min_max(sub.min, Pos2::new(sub.max.x, sub.max.y - 3.0)), 6.0, mix(BG, SURFACE, 0.5));
                // Mini toggle.
                let mini = Rect::from_min_size(Pos2::new(sub.max.x - c.toggle_w - 8.0, sub.center().y - 8.0), Vec2::new(c.toggle_w * 0.78, 17.0));
                let mt = ui.ctx().animate_value_with_time(Id::new(route.id.clone()).with("tg"), if route_enabled { 1.0 } else { 0.0 }, 0.16);
                draw_switch(&sub_painter, mini, mt, 0.0);
                // Source badge.
                sub_painter.text(Pos2::new(sub.min.x + 14.0, sub.center().y - 7.0), Align2::LEFT_TOP,
                    &route.source, FontId::new(11.0, f_medium()), ACCENT);
                // State word.
                let state = view::display_state(route);
                let state_color = if route_enabled { GOOD } else { DANGER };
                let stw = measure(ui, state, FontId::new(11.0, f_medium()));
                sub_painter.text(Pos2::new(sub.min.x + 230.0, sub.center().y - 7.0), Align2::LEFT_TOP, state, FontId::new(11.0, f_medium()), state_color);
                let _ = stw;
                // Location + command in mono, clipped.
                let zone = Rect::from_min_max(Pos2::new(sub.min.x + 320.0, sub.min.y + 4.0), Pos2::new(mini.min.x - 14.0, sub.max.y));
                let zone_clip = ui.painter().with_clip_rect(zone);
                let loc_galley = elide(ui, &route.location, FontId::new(11.0, f_mono()), TEXT_MUTED, zone.width());
                zone_clip.galley(Pos2::new(zone.min.x, zone.min.y), loc_galley, TEXT_MUTED);
                let cmd_galley = elide(ui, &route.command, FontId::new(11.0, f_mono()), mix(TEXT_MUTED, BG, 0.25), zone.width());
                zone_clip.galley(Pos2::new(zone.min.x, zone.min.y + 16.0), cmd_galley, TEXT_MUTED);
            }
        }

        // Interaction: chevron → expand, switch → toggle, elsewhere → select.
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let chevron_zone = Rect::from_min_size(Pos2::new(rect.min.x + c.name_x - 30.0, rect.min.y), Vec2::new(26.0, c.row_h));
                let toggle_zone = Rect::from_min_max(Pos2::new(track.min.x - 10.0, rect.min.y), Pos2::new(track.max.x + 10.0, rect.min.y + c.row_h));
                if row.routes.len() > 1 && chevron_zone.contains(pos) {
                    if self.expanded.contains(&row.key) { self.expanded.remove(&row.key); } else { self.expanded.insert(row.key.clone()); }
                } else if toggle_zone.contains(pos) {
                    self.toggle_row(ctx, index);
                } else {
                    self.selected_key = Some(row.key.clone());
                }
            }
        }
        if response.double_clicked() {
            self.open_edit(ctx, row);
        }
        // Sub-row hit zones (per-route toggle).
        if expanded && response.clicked() {
            for (i, route) in row.routes.iter().enumerate() {
                let sub_top = rect.min.y + c.row_h + c.sub_h * i as f32;
                let mini_x = rect.min.x + width - c.margin - c.toggle_w * 0.78 - 8.0;
                let mini_zone = Rect::from_min_size(Pos2::new(mini_x - 8.0, sub_top + 4.0), Vec2::new(c.toggle_w * 0.78 + 16.0, c.sub_h - 4.0));
                if response.interact_pointer_pos().map(|p| mini_zone.contains(p)).unwrap_or(false) {
                    self.toggle_route(ctx, route.clone());
                    break;
                }
            }
        }
        let row_clone = row.clone();
        let ctx2 = ctx.clone();
        // Hover reveal: full name, exact command, schedule, and every source.
        let hover_full = {
            let mut lines = vec![row.display_name.clone()];
            for route in &row.routes {
                lines.push(format!("{} \u{00B7} {} \u{00B7} {}", route.source, view::display_state(route), route.location));
                lines.push(route.command.clone());
            }
            lines.join("\n")
        };
        let response = response.on_hover_text(hover_full);
        response.context_menu(|ui| {
            self.row_menu(ui, &ctx2, &row_clone);
        });
    }

    fn row_menu(&mut self, ui: &mut egui::Ui, ctx: &Context, row: &InventoryRow) {
        let enable = !row.effective_enabled();
        let candidates = view::bulk_state_candidates(row, enable);
        let verb = format!("{} {}", if enable { "Enable" } else { "Disable" },
            if row.routes.len() > 1 { format!("all {} entries", row.routes.len()) } else { String::new() });
        if ui.button(verb).clicked() {
            let index = self.rows.iter().position(|r| r.key == row.key).unwrap_or(0);
            self.toggle_row(ctx, index);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Edit\u{2026}").clicked() {
            self.open_edit(ctx, row);
            ui.close_menu();
        }
        if ui.button("Run now").clicked() {
            self.run_now(ctx, row.primary_item());
            ui.close_menu();
        }
        if ui.button("Open location").clicked() {
            self.open_location(ctx, row.primary_item());
            ui.close_menu();
        }
        if ui.button("Copy command").clicked() {
            ctx.output_mut(|o| o.copied_text = row.primary_item().command.clone());
            self.set_status("Command copied.".into(), false);
            ui.close_menu();
        }
        if ui.button("Copy startup data").clicked() {
            self.copy_selected(ctx, row);
            ui.close_menu();
        }
        if row.has_managed() {
            ui.separator();
            let item = row.primary_item().clone();
            if ui.button("Quiet (tray) mode").clicked() {
                self.set_managed_mode(ctx, &item, true);
                ui.close_menu();
            }
            if ui.button("Window mode").clicked() {
                self.set_managed_mode(ctx, &item, false);
                ui.close_menu();
            }
            if ui.button("Delete managed task").clicked() {
                self.delete_managed(ctx, &item);
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Protect disabled").clicked() {
            let mut count = 0usize;
            for item in &row.routes {
                if !item.enabled {
                    msm_state::protected_disabled::protect(item);
                    count += 1;
                }
            }
            self.set_status(format!("Protected {} disabled entries \u{2014} repair will never switch them back on.", count), false);
            ui.close_menu();
        }
        if ui.button("Hide from this list").clicked() {
            self.hide_row(ctx, row);
            ui.close_menu();
        }
        // Two-click confirm: first click arms, second click erases everything.
        let armed = PURGE_ARMED.lock().map(|k| *k == row.key).unwrap_or(false);
        let purge_label = egui::RichText::new(if armed { "Click again: DELETE EVERYTHING for this app" } else { "Delete everything \u{2014} gone for good" })
            .family(f_medium()).color(DANGER);
        if ui.add(egui::Button::new(purge_label)).clicked() {
            if armed {
                if let Ok(mut k) = PURGE_ARMED.lock() { k.clear(); }
                self.purge_row(ctx, row);
                ui.close_menu();
            } else if let Ok(mut k) = PURGE_ARMED.lock() {
                *k = row.key.clone();
            }
        }
    }

    fn hidden_view(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Hidden and purged").family(f_semi()).size(16.0).color(TEXT_MAIN));
            ui.label(egui::RichText::new("Purged entries were removed from every surface and saved here for restore. Hidden entries only disappeared from the list.").size(12.0).family(f_body()).color(TEXT_MUTED));
            ui.add_space(10.0);
            // Purge backups.
            for backup in msm_actions::purge::list_backups() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("\u{2620} {}", backup.label)).family(f_semi()).size(13.0).color(TEXT_MAIN));
                    if ui.button("Restore").clicked() {
                        let id = format!("purged-{}", backup.ticks);
                        let label = backup.label.clone();
                        self.run_job(ctx, format!("Restoring {}.", label), None, move || {
                            msm_actions::purge::restore(&id)
                                .map(|name| (format!("Restored {} \u{2014} every entry is back as Disabled; switch them on when ready.", name), None))
                        });
                    }
                });
            }
            // Hidden deny entries.
            for entry in msm_state::denied::list() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("\u{25CB} {}", if entry.label.is_empty() { entry.key.clone() } else { entry.label.clone() }))
                        .family(f_body()).size(12.5).color(TEXT_MUTED));
                    if ui.button("Unhide").clicked() {
                        let key = entry.key.clone();
                        let label = entry.label.clone();
                        self.run_job(ctx, format!("Unhiding {}.", label), None, move || {
                            msm_state::denied::remove(&key);
                            Ok::<(String, Option<UndoAction>), String>((format!("{} is back in the list.", label), None))
                        });
                    }
                });
            }
            if msm_actions::purge::list_backups().is_empty() && msm_state::denied::list().is_empty() {
                ui.add_space(18.0);
                ui.label(egui::RichText::new("Nothing is hidden. Right-click any row \u{2192} Hide from this list, or Purge everywhere.").size(13.0).family(f_body()).color(TEXT_MUTED));
            }
        });
    }

    fn status_bar(&mut self, ctx: &Context, now: f64) {
        egui::TopBottomPanel::bottom("status").frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(16, 9))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 10.0;
                let (text, is_error) = (self.status.clone(), self.status_is_error);
                let color = if is_error { DANGER } else { TEXT_MUTED };
                let undo_active = self.undo.is_some();
                let right_width = 560.0f32.min(ui.available_width() * 0.6);
                let left_budget = (ui.available_width() - right_width - 90.0).max(80.0);
                let galley = elide(ui, &text, FontId::new(12.5, f_body()), color, left_budget);
                let (galley_rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 2.0, galley.size().y), Sense::hover());
                ui.painter().galley(galley_rect.min, galley, color);
                // Undo pill.
                let pending_undo = self.undo.as_ref().map(|(_, undo, _)| undo.clone());
                if let Some(undo) = pending_undo {
                    if ui.add(egui::Button::new(egui::RichText::new("Undo").family(f_semi()).size(12.0).color(BG))
                        .fill(ACCENT).stroke(Stroke::new(1.0, ACCENT))).clicked() {
                        self.undo = None;
                        self.run_undo(ctx, undo);
                    }
                } else if !undo_active {
                    ui.add_space(64.0);
                }
                with_right_layout(ui, |ui| {
                    if self.scanning.load(Ordering::SeqCst) {
                        let pulse = ((now * 2.0).fract() * 3.0) as usize;
                        let dots = ".".repeat(pulse + 1);
                        ui.label(egui::RichText::new(format!("scanning{}", dots)).size(11.5).family(f_mono()).color(TEXT_MUTED));
                    } else {
                        ui.label(egui::RichText::new("live").size(11.5).family(f_mono()).color(GOOD));
                    }
                    let enabled = self.items.iter().filter(|x| x.enabled).count();
                    ui.label(egui::RichText::new(format!("{} entries \u{00B7} {} on \u{00B7} {} off \u{00B7} {} hidden",
                        self.items.len(), enabled, self.items.len() - enabled, self.hidden_count))
                        .size(11.5).family(f_mono()).color(TEXT_MUTED));
                });
            });
        });
    }

    fn run_undo(&mut self, ctx: &Context, undo: UndoAction) {
        match undo {
            UndoAction::Toggle { routes, now_enabled } => {
                let label = routes.first().map(|r| r.human_name().to_string()).unwrap_or_default();
                self.run_job(ctx, format!("Undoing \u{2014} {} {}.", if now_enabled { "disabling" } else { "enabling" }, label), None, move || {
                    let mut applied = 0usize;
                    let mut error = String::new();
                    for route in &routes {
                        let wanted = !now_enabled;
                        let result = if wanted == route.enabled { Ok(()) }
                            else if wanted { msm_actions::enable_forced(route) } else { msm_actions::disable_forced(route) };
                        match result { Ok(()) => applied += 1, Err(e) => error = e }
                    }
                    if applied > 0 {
                        Ok((format!("Undone \u{2014} {} {}.", applied, if applied == 1 { "entry" } else { "entries" }), None))
                    } else {
                        Err(format!("Undo failed: {}", error))
                    }
                });
            }
            UndoAction::AddManaged { task } => {
                self.run_job(ctx, "Undoing the added startup.", None, move || {
                    msm_actions::managed::delete_managed_task(&task)
                        .map(|_| ("Undone \u{2014} the added startup is gone.".to_string(), None))
                });
            }
            UndoAction::DeleteManaged { location, xml } => {
                self.run_job(ctx, "Undoing the deleted task.", None, move || {
                    msm_core::managed::restore_task_xml(&location, &xml)
                        .map(|_| ("Undone \u{2014} the task is back.".to_string(), None))
                });
            }
            UndoAction::Purge { backup_id, label } => {
                self.run_job(ctx, format!("Restoring {}.", label), None, move || {
                    msm_actions::purge::restore(&backup_id)
                        .map(|name| (format!("Restored {} \u{2014} every entry is back as Disabled.", name), None))
                });
            }
            UndoAction::EditRollback { item, name, target, args, quiet } => {
                self.run_job(ctx, "Undoing the edit.", None, move || {
                    msm_actions::managed::edit_startup(&item, &name, &target, &args, quiet)
                        .map(|_| ("Undone \u{2014} the previous settings are back.".to_string(), None))
                });
            }
            UndoAction::Hidden { keys, label } => {
                self.run_job(ctx, format!("Unhiding {}.", label), None, move || {
                    for key in &keys { msm_state::denied::remove(key); }
                    Ok::<(String, Option<UndoAction>), String>((format!("{} is back in the list.", label), None))
                });
            }
        }
    }

    fn open_edit(&mut self, ctx: &Context, row: &InventoryRow) {
        let item = row.primary_item().clone();
        let (target, arguments) = msm_actions::resolve_launch_target_or_fail(&item)
            .unwrap_or_else(|_| split_command(&item.command));
        self.edit_dialog = Some(EditDialog {
            can_mode: msm_actions::managed::can_mutate_startup_mode(&item),
            item,
            name: row.display_name.clone(),
            program: target,
            arguments,
            quiet: false,
        });
        let _ = ctx;
    }

    fn dialogs(&mut self, ctx: &Context) {
        // Add dialog.
        let mut open_add = self.add_dialog.is_some();
        if open_add {
            let mut dialog = self.add_dialog.take().unwrap_or_default();
            let mut close = false;
            egui::Window::new("Add startup \u{2014} runs once at every sign-in")
                .collapsible(false).resizable(false).min_width(560.0)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Paste the full path of any program \u{2014} everything else fills itself.").size(12.5).family(f_body()).color(TEXT_MUTED));
                    ui.add_space(8.0);
                    egui::Grid::new("add-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Name (optional)");
                        ui.text_edit_singleline(&mut dialog.name);
                        ui.end_row();
                        ui.label("Program");
                        ui.add(egui::TextEdit::singleline(&mut dialog.program).font(egui::TextStyle::Monospace).desired_width(380.0));
                        ui.end_row();
                        ui.label("Arguments");
                        ui.add(egui::TextEdit::singleline(&mut dialog.arguments).font(egui::TextStyle::Monospace).desired_width(380.0));
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Paste path from clipboard").clicked() {
                            let text = read_clipboard_text();
                            if !text.trim().is_empty() {
                                dialog.program = text.trim().trim_matches('"').to_string();
                                if dialog.name.trim().is_empty() {
                                    dialog.name = std::path::Path::new(&dialog.program).file_stem()
                                        .map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                                }
                            }
                        }
                        ui.checkbox(&mut dialog.quiet, "Start quietly in the tray");
                    });
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Add").family(f_semi()).size(13.0).color(BG)).fill(ACCENT)).clicked() {
                            let program = dialog.program.clone();
                            let args = dialog.arguments.clone();
                            let quiet = dialog.quiet;
                            let name = {
                                let n = dialog.name.trim().to_string();
                                if n.is_empty() {
                                    std::path::Path::new(&program).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                                } else { n }
                            };
                            self.run_job(ctx, "Adding\u{2026}", None, move || {
                                msm_actions::managed::add_managed_startup(&name, &program, &args, quiet, true)
                                    .map(|task| (format!("Added {} \u{00B7} runs once at every sign-in.", name),
                                        Some(UndoAction::AddManaged { task })))
                            });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() { close = true; }
                    });
                });
            let _ = &mut open_add;
            if !close { self.add_dialog = Some(dialog); } else { self.add_dialog = None; }
        }
        // Edit dialog.
        let mut open_edit = self.edit_dialog.is_some();
        if open_edit {
            let mut dialog = self.edit_dialog.take().unwrap();
            let original = (dialog.item.clone(), dialog.name.clone(), dialog.program.clone(), dialog.arguments.clone(), dialog.quiet);
            let mut close = false;
            egui::Window::new("Edit startup")
                .collapsible(false).resizable(false).min_width(560.0)
                .show(ctx, |ui| {
                    egui::Grid::new("edit-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut dialog.name);
                        ui.end_row();
                        ui.label("Program");
                        ui.add(egui::TextEdit::singleline(&mut dialog.program).font(egui::TextStyle::Monospace).desired_width(380.0));
                        ui.end_row();
                        ui.label("Arguments");
                        ui.add(egui::TextEdit::singleline(&mut dialog.arguments).font(egui::TextStyle::Monospace).desired_width(380.0));
                        ui.end_row();
                    });
                    if dialog.can_mode {
                        ui.checkbox(&mut dialog.quiet, "Start quietly in the tray");
                    }
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Save").family(f_semi()).size(13.0).color(BG)).fill(ACCENT)).clicked() {
                            let (item, name, program, args, quiet) = (dialog.item.clone(), dialog.name.trim().to_string(),
                                dialog.program.trim().to_string(), dialog.arguments.trim().to_string(), dialog.quiet);
                            let old = original;
                            self.run_job(ctx, "Saving\u{2026}", None, move || {
                                msm_actions::managed::edit_startup(&item, &name, &program, &args, quiet)
                                    .map(|_| (format!("Updated {}.", name),
                                        Some(UndoAction::EditRollback { item: old.0.clone(), name: old.1, target: old.2, args: old.3, quiet: old.4 })))
                            });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() { close = true; }
                    });
                });
            let _ = &mut open_edit;
            if !close { self.edit_dialog = Some(dialog); } else { self.edit_dialog = None; }
        }
    }
}

fn with_right_layout(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
}

fn entry_hint(item: &StartupItem) -> String {
    let command = item.command.trim();
    if command.is_empty() { item.location.clone() } else { command.to_string() }
}

fn short_source(source: &str) -> String {
    source.replace("Registry ", "Registry ").replace("Scheduled Task (Any Trigger)", "Task (any trigger)")
}

fn draw_switch(painter: &egui::Painter, rect: Rect, t: f32, hover: f32) {
    let on = t;
    let track_fill = if on > 0.5 { mix(GOOD, BG, 0.68) } else { SURFACE_2 };
    let track_fill = mix(track_fill, TEXT_MAIN, hover * 0.04);
    painter.rect_filled(rect, rect.height() / 2.0, track_fill);
    painter.rect_stroke(rect, rect.height() / 2.0, Stroke::new(1.0, if on > 0.5 { mix(GOOD, BG, 0.25) } else { mix(BORDER, TEXT_MAIN, 0.10) }), egui::StrokeKind::Middle);
    let knob_r = rect.height() / 2.0 - 3.0;
    let knob_x = rect.min.x + 4.0 + (rect.width() - 8.0 - knob_r * 2.0) * on;
    let knob_color = if on > 0.5 { GOOD } else { TEXT_MUTED };
    painter.circle_filled(Pos2::new(knob_x + knob_r, rect.center().y), knob_r, knob_color);
    painter.circle_filled(Pos2::new(knob_x + knob_r, rect.center().y), knob_r - 2.0, mix(knob_color, if on > 0.5 { GOOD } else { SURFACE_2 }, 0.35));
}
