//! printcraft-ui-egui — the first PrintCraft shell (L7).
//!
//! Layout grammar follows plan/acrobat/02-ui-ux.md §1: tab strip, mode bar, left tool panel,
//! floating quick-action bar, document area, right panel + right rail with page navigation.
//! Everything here is presentation: documents, rendering and the tool catalogue live in
//! `printcraft-engine`.

mod canvas;
mod chrome;
mod dialogs;
mod home;
mod icon_data;
pub mod icons;
mod palette;
mod panels;
pub mod theme;
mod widgets;

use printcraft_engine::{DocId, Session};

pub use canvas::DocView;
use theme::ThemeKind;

/// Top-level workspace modes (Acrobat's mode bar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AllTools,
    Read,
    Edit,
    Convert,
    Sign,
}

/// What the left panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftPanel {
    AllTools,
    Tool(&'static str),
}

/// Right-hand panels, opened from the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightPanel {
    Comments,
    Bookmarks,
    Pages,
    Fields,
    Layers,
    Attachments,
}

/// Quick-action bar tools (the vertical floating strip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickTool {
    Select,
    Hand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    Properties(PropsTab),
    About,
    Shortcuts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropsTab {
    Description,
    Security,
    Advanced,
}

pub struct RecentFile {
    pub name: String,
    pub path: String,
    pub pages: usize,
    pub size: usize,
}

pub struct PrintCraftApp {
    pub session: Session,
    pub views: Vec<DocView>,
    /// `None` shows the Home tab.
    pub active: Option<usize>,
    pub mode: Mode,
    pub left: LeftPanel,
    pub left_open: bool,
    pub right: Option<RightPanel>,
    pub quick_tool: QuickTool,
    pub theme: ThemeKind,
    pub dialog: Option<Dialog>,
    pub palette_open: bool,
    pub palette_query: String,
    pub all_tools_expanded: bool,
    pub recent: Vec<RecentFile>,
    pub toast: Option<(String, f64)>,
    /// Whether the macOS title bar is drawn by us (traffic lights over our tab strip).
    pub integrated_titlebar: bool,
    pending_theme: Option<ThemeKind>,
    styled: bool,
    fonts_ready: bool,
}

impl Default for PrintCraftApp {
    fn default() -> Self {
        Self::new()
    }
}

impl PrintCraftApp {
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            views: Vec::new(),
            active: None,
            mode: Mode::AllTools,
            left: LeftPanel::AllTools,
            left_open: true,
            right: None,
            quick_tool: QuickTool::Select,
            theme: ThemeKind::Light,
            dialog: None,
            palette_open: false,
            palette_query: String::new(),
            all_tools_expanded: false,
            recent: Vec::new(),
            toast: None,
            integrated_titlebar: false,
            pending_theme: None,
            styled: false,
            fonts_ready: false,
        }
    }

    /// Open a document and make it the active tab.
    pub fn open_bytes(&mut self, name: &str, path: Option<String>, bytes: Vec<u8>) -> Result<(), String> {
        let size = bytes.len();
        let id = self.session.open(name, path.clone(), bytes).map_err(|e| e.to_string())?;
        let doc = self.session.get(id).expect("just opened");
        let pages = doc.info.pages.len();
        // Acrobat opens straight to the Comments panel when a document has comments.
        if self.right.is_none() {
            self.right = if !doc.info.annotations.is_empty() {
                Some(RightPanel::Comments)
            } else if !doc.info.outline.is_empty() {
                Some(RightPanel::Bookmarks)
            } else {
                None
            };
        }
        self.views.push(DocView::new(id, &doc.info));
        self.active = Some(self.views.len() - 1);
        if let Some(p) = path {
            self.recent.retain(|r| r.path != p);
            self.recent.insert(0, RecentFile { name: name.to_string(), path: p, pages, size });
            self.recent.truncate(12);
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: &str) {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        match std::fs::read(path) {
            Ok(bytes) => {
                if let Err(e) = self.open_bytes(&name, Some(path.to_string()), bytes) {
                    self.notify(format!("Couldn't open {name}: {e}"));
                }
            }
            Err(e) => self.notify(format!("Couldn't read {name}: {e}")),
        }
    }

    pub fn open_dialog(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
            self.open_path(&p.to_string_lossy());
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.views.len() {
            return;
        }
        let id = self.views.remove(index).id;
        self.session.close(id);
        self.active = match self.active {
            _ if self.views.is_empty() => None,
            Some(a) if a >= self.views.len() => Some(self.views.len() - 1),
            other => other,
        };
    }

    pub fn active_ids(&self) -> Option<(usize, DocId)> {
        self.active.and_then(|i| self.views.get(i).map(|v| (i, v.id)))
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), 0.0));
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, kind: ThemeKind) {
        self.theme = kind;
        theme::apply(ctx, kind);
    }

    /// Run a catalogue command. Commands that aren't implemented yet say which milestone ships them.
    pub fn run_command(&mut self, command: &str) {
        match command {
            "page.organize" => {
                if let Some(i) = self.active {
                    self.views[i].organize = !self.views[i].organize;
                } else {
                    self.notify("Open a document first");
                }
            }
            "comment.list" => self.right = Some(RightPanel::Comments),
            "form.fields" => self.right = Some(RightPanel::Fields),
            "protect.properties" => self.dialog = Some(Dialog::Properties(PropsTab::Security)),
            other => {
                let when = printcraft_engine::catalog::TOOL_GROUPS
                    .iter()
                    .flat_map(|g| g.sections.iter().flat_map(|s| s.items.iter()))
                    .find(|i| i.command == other)
                    .map(|i| match i.availability {
                        printcraft_engine::catalog::Availability::Planned(m) => format!("ships in milestone {m}"),
                        printcraft_engine::catalog::Availability::Provider => "needs an AI provider (off by default)".to_string(),
                        printcraft_engine::catalog::Availability::Ready => "is available".to_string(),
                    })
                    .unwrap_or_else(|| "is not available yet".into());
                self.notify(format!("`{other}` {when}"));
            }
        }
    }

    /// Apply a named view option (`--page 3`, `--panel bookmarks`, `--theme dark`, …).
    ///
    /// This is the seed of the UI control channel (M3.9): the same verbs become `ui.set` calls.
    pub fn set_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        let view = self.active.and_then(|i| self.views.get_mut(i));
        match (key, view) {
            ("theme", _) => self.pending_theme = Some(if value == "dark" { ThemeKind::Dark } else { ThemeKind::Light }),
            ("panel", _) => {
                self.right = match value {
                    "comments" => Some(RightPanel::Comments),
                    "bookmarks" => Some(RightPanel::Bookmarks),
                    "pages" => Some(RightPanel::Pages),
                    "fields" => Some(RightPanel::Fields),
                    "layers" => Some(RightPanel::Layers),
                    "attachments" => Some(RightPanel::Attachments),
                    "none" => None,
                    other => return Err(format!("unknown panel {other}")),
                }
            }
            ("mode", _) => {
                self.mode = match value {
                    "read" => Mode::Read,
                    "edit" => Mode::Edit,
                    "convert" => Mode::Convert,
                    "sign" => Mode::Sign,
                    _ => Mode::AllTools,
                }
            }
            ("tool", _) => {
                let g = printcraft_engine::catalog::group(value).ok_or_else(|| format!("unknown tool {value}"))?;
                self.left = LeftPanel::Tool(g.id);
                self.left_open = true;
            }
            ("left", _) => self.left_open = value != "closed",
            ("home", _) => self.active = None,
            ("dialog", _) => {
                self.dialog = match value {
                    "properties" => Some(Dialog::Properties(PropsTab::Description)),
                    "shortcuts" => Some(Dialog::Shortcuts),
                    _ => Some(Dialog::About),
                }
            }
            ("palette", _) => {
                self.palette_open = true;
                self.palette_query = value.to_string();
            }
            ("page", Some(v)) => v.go_to_page(value.parse::<usize>().map_err(|e| e.to_string())?.saturating_sub(1)),
            ("zoom", Some(v)) => v.set_zoom(value.trim_end_matches('%').parse::<f32>().map_err(|e| e.to_string())? / 100.0),
            ("layout", Some(v)) => {
                v.layout = match value {
                    "two-up" => canvas::PageLayout::TwoUp,
                    "single" => canvas::PageLayout::Single,
                    _ => canvas::PageLayout::Continuous,
                }
            }
            ("organize", Some(v)) => v.organize = value != "off",
            ("fields", Some(v)) => v.highlight_fields = value != "off",
            (k, None) if ["page", "zoom", "layout", "organize", "fields"].contains(&k) => return Err(format!("`{k}` needs an open document")),
            (other, _) => return Err(format!("unknown option {other}")),
        }
        Ok(())
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(cmd(Key::O)) {
            self.open_dialog();
        }
        if pressed(cmd(Key::K)) {
            self.palette_open = !self.palette_open;
        }
        if pressed(cmd(Key::D)) && self.active.is_some() {
            self.dialog = Some(Dialog::Properties(PropsTab::Description));
        }
        if pressed(cmd(Key::W))
            && let Some(i) = self.active
        {
            self.close_tab(i);
        }
        if pressed(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::CTRL, Key::H)) {
            self.mode = if self.mode == Mode::Read { Mode::AllTools } else { Mode::Read };
        }
        if let Some(i) = self.active {
            canvas::shortcuts(&mut self.views[i], ctx);
        }
    }
}

impl eframe::App for PrintCraftApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.styled {
            egui_extras::install_image_loaders(ctx);
            theme::install_fonts(ctx);
            theme::apply(ctx, self.theme);
            self.styled = true;
        } else {
            self.fonts_ready = true;
        }
        if let Some(k) = self.pending_theme.take() {
            self.set_theme(ctx, k);
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let p = f.path().to_string_lossy().into_owned();
                if !p.is_empty() {
                    self.open_path(&p);
                    continue;
                }
            }
            let name = f.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "dropped.pdf".into());
            match f.bytes() {
                Ok(bytes) => {
                    if let Err(e) = self.open_bytes(&name, None, bytes) {
                        self.notify(format!("Couldn't open {name}: {e}"));
                    }
                }
                Err(e) => self.notify(format!("Couldn't read {name}: {e}")),
            }
        }
        self.shortcuts(ctx);
        // Pull finished renders into textures for every open document.
        for view in &mut self.views {
            if let Some(doc) = self.session.get(view.id) {
                view.receive(ctx, &doc.renderer);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Fonts registered via set_fonts only take effect next frame; named families would panic now.
        if !self.fonts_ready {
            ctx.request_repaint();
            return;
        }
        chrome::tab_strip(self, ui);
        chrome::mode_bar(self, ui);
        if self.active.is_some() {
            chrome::right_rail(self, ui);
            if self.right.is_some() && self.mode != Mode::Read {
                panels::right_panel(self, ui);
            }
        }
        if self.left_open && self.mode != Mode::Read {
            panels::left_panel(self, ui);
        }
        let t = theme::Tokens::get(&ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| match self.active {
            None => home::show(self, ui),
            Some(i) => canvas::document_area(self, i, ui),
        });
        palette::show(self, &ctx);
        dialogs::show(self, &ctx);
        widgets::toast(self, &ctx);
    }
}
