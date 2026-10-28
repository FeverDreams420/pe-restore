//! The tool's window: pick the game folder, pick a crash report, write the save.

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, FontSelection, Frame, Layout, Margin, RichText, Sense,
    TextStyle, Theme, Ui, Visuals, vec2,
};
use pe_save_editor::tables::{map_location_descriptive, save_header_location_name};

use crate::report::{self, CrashReport};
use crate::{card, save};

/// Memory card 1 slot the save goes to (0-based).
const SLOT: usize = 0;
/// Remembers the chosen game folder, next to pe-restore.exe.
const FOLDER_FILE: &str = "pe-restore-folder.txt";

pub fn run(start: Option<PathBuf>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PE Restore")
            .with_inner_size([720.0, 800.0])
            .with_min_inner_size([560.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PE Restore",
        options,
        Box::new(move |cc| {
            set_style(&cc.egui_ctx);
            Ok(Box::new(App::new(start)))
        }),
    )
}

// ---- Look ------------------------------------------------------------------

/// Colours on top of egui's own light and dark themes.
struct Palette {
    accent: Color32,
    ok: Color32,
    info: Color32,
    card: Color32,
    card_stroke: Color32,
}

fn palette(visuals: &Visuals) -> Palette {
    if visuals.dark_mode {
        Palette {
            accent: Color32::from_rgb(0xD9, 0x48, 0x41),
            ok: Color32::from_rgb(0x4C, 0xC3, 0x6E),
            info: Color32::from_rgb(0x6C, 0xA8, 0xF0),
            card: Color32::from_gray(33),
            card_stroke: Color32::from_gray(52),
        }
    } else {
        Palette {
            accent: Color32::from_rgb(0xB8, 0x2E, 0x2A),
            ok: Color32::from_rgb(0x2E, 0x8B, 0x4A),
            info: Color32::from_rgb(0x2B, 0x6C, 0xC4),
            card: Color32::from_gray(252),
            card_stroke: Color32::from_gray(218),
        }
    }
}

fn set_style(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            style.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
            style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
            style.text_styles.insert(TextStyle::Button, FontId::proportional(14.5));
            style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
            style.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
            style.spacing.item_spacing = vec2(8.0, 6.0);
            style.spacing.button_padding = vec2(10.0, 5.0);

            let v = &mut style.visuals;
            let accent = palette(v).accent;
            v.panel_fill = Color32::from_gray(if dark { 24 } else { 240 });
            v.hyperlink_color = accent;
            v.selection.bg_fill =
                if dark { Color32::from_rgb(0x5C, 0x24, 0x22) } else { Color32::from_rgb(0xF5, 0xD6, 0xD4) };
            v.selection.stroke = (1.0, if dark { Color32::WHITE } else { Color32::from_rgb(0x5A, 0x10, 0x0E) }).into();
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                // More than 4 turns the checkbox into a circle.
                w.corner_radius = CornerRadius::same(4);
            }
        });
    }
}

/// A numbered card for one step of the restore.
fn section<R>(ui: &mut Ui, step: u8, done: bool, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.visuals());
    Frame::new()
        .fill(p.card)
        .stroke((1.0, p.card_stroke))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 12.0, if done { p.ok } else { p.accent });
                let mark = if done { "✔".to_string() } else { step.to_string() };
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    mark,
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );
                ui.label(RichText::new(title).size(16.5).strong());
            });
            ui.add_space(8.0);
            add(ui)
        })
        .inner
}

#[derive(Clone, Copy)]
enum Tone {
    Ok,
    Warn,
    Error,
    Info,
}

/// A tinted box with an icon, for results, warnings and notes.
fn banner(ui: &mut Ui, tone: Tone, add: impl FnOnce(&mut Ui)) {
    let p = palette(ui.visuals());
    let (color, icon) = match tone {
        Tone::Ok => (p.ok, "✔"),
        Tone::Warn => (ui.visuals().warn_fg_color, "⚠"),
        Tone::Error => (ui.visuals().error_fg_color, "✖"),
        Tone::Info => (p.info, "ℹ"),
    };
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke((1.0, color.gamma_multiply(0.55)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(icon).color(color).size(16.0));
                ui.vertical(add);
            });
        });
}

fn banner_text(ui: &mut Ui, tone: Tone, text: impl Into<String>) {
    banner(ui, tone, |ui| {
        ui.label(text.into());
    });
}

fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    let accent = palette(ui.visuals()).accent;
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(15.5))
            .fill(accent)
            .min_size(vec2(200.0, 36.0)),
    )
}

/// A path in a sunken box, like a read-only text field.
fn path_box(ui: &mut Ui, path: &Path) {
    Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(path.display().to_string()).monospace()).wrap());
        },
    );
}

// ---- State -----------------------------------------------------------------

struct Entry {
    path: PathBuf,
    label: String,
    report: Result<CrashReport, String>,
}

impl Entry {
    fn read(path: PathBuf) -> Self {
        let report = fs::read(&path)
            .map_err(|e| format!("Can't open it: {e}"))
            .and_then(|bytes| report::parse(&String::from_utf8_lossy(&bytes)));
        Entry { label: label_for(&path), path, report }
    }
}

/// "crash-2026-10-27_17-46-13.txt" -> "2026-10-27 17:46:13".
fn label_for(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let stamp = name.strip_prefix("crash-").and_then(|n| n.strip_suffix(".txt"));
    match stamp.and_then(|s| s.split_once('_')) {
        Some((date, time)) => format!("{date} {}", time.replace('-', ":")),
        None => name,
    }
}

/// Where the port writes its crash reports: `<game folder>\logs`, else the folder itself.
fn reports_dir(game_dir: &Path) -> PathBuf {
    let logs = game_dir.join("logs");
    if logs.is_dir() { logs } else { game_dir.to_path_buf() }
}

enum Message {
    Written { title: String, path: PathBuf, backups: Vec<PathBuf>, warning: Option<String> },
    Failed(String),
    Cancelled,
}

enum Action {
    Select(usize),
    ChooseFolder,
    OpenFile,
    Refresh,
    SetStrong(bool),
    Write,
    AskReplace,
    CancelReplace,
}

struct App {
    game_dir: Option<PathBuf>,
    reports: Vec<Entry>,
    selected: Option<usize>,
    strong: bool,
    /// Card 1 the game reads: the settings-folder one once 0.6 or newer has
    /// moved to it, else the one inside the game folder.
    card_dir: Option<PathBuf>,
    /// Card 1 inside the game folder when the save goes to the settings-folder
    /// card: builds older than 0.6 still read this one.
    game_folder_card: Option<PathBuf>,
    /// Card titles of the files already in the slot.
    slot_saves: Vec<String>,
    /// The normal save file in the slot, for its mod data (`save::old_tail`).
    slot_bytes: Option<Vec<u8>>,
    confirm_replace: bool,
    message: Option<Message>,
}

impl App {
    fn new(start: Option<PathBuf>) -> Self {
        let mut app = App {
            game_dir: None,
            reports: Vec::new(),
            selected: None,
            strong: false,
            card_dir: None,
            game_folder_card: None,
            slot_saves: Vec::new(),
            slot_bytes: None,
            confirm_replace: false,
            message: None,
        };
        let (dir, file) = match start {
            Some(path) if path.is_dir() => (Some(path), None),
            other => (remembered_folder(), other),
        };
        if let Some(dir) = dir {
            app.set_game_dir(dir);
        }
        if let Some(file) = file {
            app.open_file(file);
        }
        app
    }

    fn set_game_dir(&mut self, dir: PathBuf) {
        remember_folder(&dir);
        self.game_dir = Some(dir);
        self.message = None;
        self.rescan();
    }

    /// Re-reads the reports and the card slot from disk.
    fn rescan(&mut self) {
        // Looked up again each time: starting the game in between can move the card.
        let game_folder_card = self.game_dir.as_deref().map(card::card1_in);
        match card::settings_card1().filter(|_| game_folder_card.is_some()) {
            Some(settings) => {
                self.card_dir = Some(settings);
                self.game_folder_card = game_folder_card;
            }
            None => {
                self.card_dir = game_folder_card;
                self.game_folder_card = None;
            }
        }
        self.scan_reports();
        self.refresh_slot();
    }

    /// Lists the crash-*.txt reports, newest first, and selects the newest usable one.
    fn scan_reports(&mut self) {
        self.reports.clear();
        self.selected = None;
        self.confirm_replace = false;
        let Some(dir) = &self.game_dir else { return };
        let mut paths: Vec<PathBuf> = fs::read_dir(reports_dir(dir))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        let name = p.file_name().unwrap_or_default().to_string_lossy();
                        p.is_file() && name.starts_with("crash-") && name.ends_with(".txt")
                    })
                    .collect()
            })
            .unwrap_or_default();
        paths.sort_by(|a, b| b.cmp(a));
        self.reports = paths.into_iter().map(Entry::read).collect();
        self.selected = self.reports.iter().position(|e| e.report.is_ok());
    }

    fn open_file(&mut self, path: PathBuf) {
        let index = self.reports.iter().position(|e| e.path == path).unwrap_or_else(|| {
            self.reports.insert(0, Entry::read(path));
            0
        });
        self.select(index);
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.confirm_replace = false;
        self.message = None;
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.selected.and_then(|i| self.reports.get(i))
    }

    fn refresh_slot(&mut self) {
        self.slot_saves = match &self.card_dir {
            Some(dir) => card::slot_files(dir, SLOT).iter().map(|f| card::describe(f)).collect(),
            None => Vec::new(),
        };
        self.slot_bytes = self.card_dir.as_ref().and_then(|dir| fs::read(dir.join(save::file_name(SLOT))).ok());
    }

    /// The warning about the mods' data for the selected report, if any.
    fn tail_warning(&self) -> Option<String> {
        let report = self.selected_entry()?.report.as_ref().ok()?;
        let old = self.slot_bytes.as_deref().and_then(save::old_tail);
        save::tail_warning(report, &save::tail_from(report, old))
    }

    fn write(&mut self) {
        self.confirm_replace = false;
        self.message = Some(self.try_write().unwrap_or_else(Message::Failed));
        self.refresh_slot();
    }

    fn try_write(&self) -> Result<Message, String> {
        let entry = self.selected_entry().ok_or("Pick a crash report first.")?;
        let report = entry.report.as_ref().map_err(|e| e.clone())?;
        let dir = self.card_dir.as_ref().ok_or("Choose the game folder first.")?;
        // Read again now: the mods' data is kept from the slot's save as it is at this moment.
        let old_bytes = fs::read(dir.join(save::file_name(SLOT))).ok();
        let old = old_bytes.as_deref().and_then(save::old_tail);
        let warning = save::tail_warning(report, &save::tail_from(report, old));
        let mut slot = save::build(report, SLOT, old).map_err(|e| format!("Can't build the save: {e}"))?;
        if self.strong {
            save::apply_test_setup(&mut slot).map_err(|e| format!("Can't change the save: {e}"))?;
        }
        let bytes = save::to_bytes(&mut slot);
        let written =
            card::write_slot(dir, SLOT, &slot.filename, &bytes).map_err(|e| format!("Can't write the save: {e}"))?;
        Ok(Message::Written { title: slot.header_title.clone(), path: written.path, backups: written.backups, warning })
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Select(i) => self.select(i),
            Action::ChooseFolder => {
                let mut dialog = rfd::FileDialog::new().set_title("Choose the Parasite Eve game folder");
                if let Some(dir) = &self.game_dir {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.set_game_dir(dir);
                }
            }
            Action::OpenFile => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Open a Parasite Eve crash report")
                    .add_filter("Crash report", &["txt"]);
                if let Some(dir) = &self.game_dir {
                    dialog = dialog.set_directory(reports_dir(dir));
                }
                if let Some(path) = dialog.pick_file() {
                    self.open_file(path);
                }
            }
            Action::Refresh => {
                self.rescan();
                self.message = None;
            }
            Action::SetStrong(on) => self.strong = on,
            Action::Write => self.write(),
            Action::AskReplace => self.confirm_replace = true,
            Action::CancelReplace => {
                self.confirm_replace = false;
                self.message = Some(Message::Cancelled);
            }
        }
    }

    // ---- Drawing -------------------------------------------------------------

    fn header_ui(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let accent = palette(ui.visuals()).accent;
            ui.label(RichText::new("PE Restore").heading().strong().color(accent));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::widgets::global_theme_preference_switch(ui);
            });
        });
        ui.label(RichText::new("Turns a crash report from the PC port into a save on memory card 1.").weak());
    }

    fn folder_ui(&self, ui: &mut Ui, action: &mut Option<Action>) {
        section(ui, 1, self.game_dir.is_some(), "Game folder", |ui| {
            match &self.game_dir {
                Some(dir) => path_box(ui, dir),
                None => {
                    ui.label(
                        RichText::new(
                            "The folder with the game's exe. You can also drop it, or a report, on this window.",
                        )
                        .weak(),
                    );
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("📂  Choose folder…").clicked() {
                    *action = Some(Action::ChooseFolder);
                }
                if ui.button("📄  Open a report file…").clicked() {
                    *action = Some(Action::OpenFile);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("🔄  Refresh").on_hover_text("Read the reports and the memory card again").clicked()
                    {
                        *action = Some(Action::Refresh);
                    }
                });
            });
        });
    }

    fn reports_ui(&self, ui: &mut Ui, action: &mut Option<Action>) {
        let usable = self.selected_entry().is_some_and(|e| e.report.is_ok());
        section(ui, 2, usable, "Crash report", |ui| {
            if self.reports.is_empty() {
                let text = match &self.game_dir {
                    Some(dir) => format!("No crash reports (crash-*.txt) in {}.", reports_dir(dir).display()),
                    None => "Choose the game folder above, or open a report file.".into(),
                };
                ui.label(RichText::new(text).weak());
                return;
            }

            Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(6).inner_margin(Margin::same(4)).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical().id_salt("reports").max_height(190.0).auto_shrink([false, true]).show(
                        ui,
                        |ui| {
                            ui.spacing_mut().button_padding = vec2(10.0, 6.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                                for (i, entry) in self.reports.iter().enumerate() {
                                    let row =
                                        egui::SelectableLabel::new(self.selected == Some(i), report_row(ui, entry));
                                    let mut row = ui.add(row);
                                    if let Err(e) = &entry.report {
                                        row = row.on_hover_text(e);
                                    }
                                    if row.clicked() {
                                        *action = Some(Action::Select(i));
                                    }
                                }
                            });
                        },
                    );
                },
            );

            let Some(entry) = self.selected_entry() else { return };
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(entry.path.display().to_string()).small().weak()).wrap());
            match &entry.report {
                Err(e) => banner_text(ui, Tone::Error, format!("Can't use this report: {e}")),
                Ok(r) if save_header_location_name(r.room, false).is_empty() => banner_text(
                    ui,
                    Tone::Warn,
                    "This room is a cutscene or special map. It's not known yet whether the game loads \
                     a save made there cleanly.",
                ),
                Ok(_) => {}
            }
            if let Ok(CrashReport { seconds_before: Some(s), .. }) = &entry.report {
                banner_text(
                    ui,
                    Tone::Info,
                    format!("The save is from the start of this room, {s} seconds before the crash."),
                );
            }
        });
    }

    fn write_ui(&self, ui: &mut Ui, action: &mut Option<Action>) {
        let done = matches!(self.message, Some(Message::Written { .. }));
        section(ui, 3, done, "Write the save", |ui| {
            if !self.selected_entry().is_some_and(|e| e.report.is_ok()) {
                ui.label(RichText::new("Pick a usable crash report above.").weak());
                return;
            }
            let Some(dir) = &self.card_dir else {
                banner_text(
                    ui,
                    Tone::Warn,
                    "Choose the game folder first: the program finds the game's memory card from it.",
                );
                return;
            };

            let mut strong = self.strong;
            if ui.checkbox(&mut strong, "Make Aya strong for testing").changed() {
                *action = Some(Action::SetStrong(strong));
            }
            ui.horizontal(|ui| {
                // Line up with the checkbox's text.
                ui.add_space(ui.spacing().icon_width + ui.spacing().icon_spacing);
                ui.label(
                    RichText::new(
                        "Level 99, 999 HP, a strong gun and armour in the bag and 6 Full Recovers. \
                         Nothing she carries is removed.",
                    )
                    .small()
                    .weak(),
                );
            });
            ui.add_space(8.0);

            egui::Grid::new("card").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("Memory card 1").weak());
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(dir.display().to_string()).monospace()).wrap());
                    if !dir.is_dir() {
                        ui.label(RichText::new("Will be created.").small().weak());
                    }
                });
                ui.end_row();

                ui.label(RichText::new(format!("Slot {}", SLOT + 1)).weak());
                ui.vertical(|ui| {
                    if self.slot_saves.is_empty() {
                        ui.label("Empty");
                    } else {
                        for save in &self.slot_saves {
                            ui.monospace(save);
                        }
                        ui.label(
                            RichText::new("Moved to pe-restore-backups next to bu00 when replaced, never deleted.")
                                .small()
                                .weak(),
                        );
                    }
                });
                ui.end_row();
            });

            if let Some(old) = &self.game_folder_card {
                ui.add_space(4.0);
                banner_text(
                    ui,
                    Tone::Info,
                    format!(
                        "The port (0.6 and newer) copied the game folder's card into the settings folder once \
                         and reads that card from then on, so the save goes there. Builds older than 0.6 read \
                         {} instead.",
                        old.display()
                    ),
                );
            }

            if let Some(warning) = self.tail_warning().filter(|_| !done) {
                ui.add_space(4.0);
                banner_text(ui, Tone::Warn, warning);
            }

            ui.add_space(10.0);
            if self.confirm_replace {
                banner(ui, Tone::Warn, |ui| {
                    ui.label(RichText::new(format!("Replace the save in slot {}?", SLOT + 1)).strong());
                    ui.horizontal(|ui| {
                        if primary_button(ui, "Yes, replace").clicked() {
                            *action = Some(Action::Write);
                        }
                        if ui.add(egui::Button::new("Cancel").min_size(vec2(90.0, 36.0))).clicked() {
                            *action = Some(Action::CancelReplace);
                        }
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    if primary_button(ui, &format!("💾  Write to slot {}", SLOT + 1)).clicked() {
                        *action = Some(if self.slot_saves.is_empty() { Action::Write } else { Action::AskReplace });
                    }
                    ui.label(RichText::new("Close the game first.").weak());
                });
            }
        });
    }

    fn message_ui(&self, ui: &mut Ui) {
        match &self.message {
            None => {}
            Some(Message::Written { title, path, backups, warning }) => {
                banner(ui, Tone::Ok, |ui| {
                    ui.label(RichText::new(format!("Written to slot {}: \"{title}\"", SLOT + 1)).strong());
                    ui.add(egui::Label::new(RichText::new(path.display().to_string()).monospace()).wrap());
                    for backup in backups {
                        ui.add(
                            egui::Label::new(RichText::new(format!("Old save moved to {}", backup.display())).weak())
                                .wrap(),
                        );
                    }
                    ui.add_space(2.0);
                    ui.label(format!("Start the game and load slot {} on memory card 1.", SLOT + 1));
                });
                if let Some(warning) = warning {
                    ui.add_space(4.0);
                    banner_text(ui, Tone::Warn, warning.as_str());
                }
            }
            Some(Message::Failed(text)) => banner_text(ui, Tone::Error, text),
            Some(Message::Cancelled) => banner_text(ui, Tone::Info, "Nothing written."),
        }
    }
}

/// One line of the report list: date, place, room and story day.
fn report_row(ui: &Ui, entry: &Entry) -> LayoutJob {
    let style = ui.style();
    let mut job = LayoutJob::default();
    let mut add = |text: RichText| text.append_to(&mut job, style, FontSelection::Default, Align::Center);
    match &entry.report {
        Ok(r) => {
            add(RichText::new(&entry.label).strong());
            add(RichText::new(format!("     {}", map_location_descriptive(r.room))));
            add(RichText::new(format!("     room {} · day {}", r.room, r.story_day)).weak());
        }
        Err(_) => {
            add(RichText::new(format!("⚠ {}", entry.label)).weak());
            add(RichText::new("     can't use this report").weak().italics());
        }
    }
    job
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for path in dropped {
            if path.is_dir() {
                self.set_game_dir(path);
            } else {
                self.open_file(path);
            }
        }

        let mut action = None;
        let fill = ctx.style().visuals.panel_fill;
        egui::TopBottomPanel::top("header")
            .show_separator_line(false)
            .frame(Frame::new().fill(fill).inner_margin(Margin { left: 18, right: 18, top: 14, bottom: 4 }))
            .show(ctx, |ui| self.header_ui(ui));
        egui::CentralPanel::default().frame(Frame::new().fill(fill).inner_margin(Margin::same(18))).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.folder_ui(ui, &mut action);
                ui.add_space(10.0);
                self.reports_ui(ui, &mut action);
                ui.add_space(10.0);
                self.write_ui(ui, &mut action);
                if self.message.is_some() {
                    ui.add_space(10.0);
                    self.message_ui(ui);
                }
            });
        });

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            drop_overlay(ctx);
        }
        if let Some(action) = action {
            self.apply(action);
        }
    }
}

/// Shown while files are dragged over the window.
fn drop_overlay(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
    let rect = ctx.screen_rect();
    painter.rect_filled(rect, 0, Color32::from_black_alpha(190));
    painter.rect_stroke(rect.shrink(16.0), 12, (2.0, palette(&ctx.style().visuals).accent), egui::StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "Drop a crash report or the game folder",
        FontId::proportional(20.0),
        Color32::WHITE,
    );
}

// ---- Remembered folder -------------------------------------------------------

fn folder_file() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(FOLDER_FILE))
}

fn remembered_folder() -> Option<PathBuf> {
    let text = fs::read_to_string(folder_file()?).ok()?;
    let dir = PathBuf::from(text.trim());
    dir.is_dir().then_some(dir)
}

fn remember_folder(dir: &Path) {
    if let Some(file) = folder_file() {
        let _ = fs::write(file, dir.display().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_crash_file_names_by_date() {
        assert_eq!(label_for(Path::new("logs/crash-2026-10-27_17-46-13.txt")), "2026-10-27 17:46:13");
        assert_eq!(label_for(Path::new("other.txt")), "other.txt");
    }
}
