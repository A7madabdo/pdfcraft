//! Modal dialogs: Document Properties, Keyboard Shortcuts, About.

use egui::{Align, Layout};

use crate::theme::{self, Tokens};
use printcraft_engine::Edit;

use crate::{CloseRequest, Dialog, PrintCraftApp, PropsTab, panels::human_size, widgets};

const INFO_KEYS: [&str; 4] = ["Title", "Author", "Subject", "Keywords"];

pub fn show(app: &mut PrintCraftApp, ctx: &egui::Context) {
    password(app, ctx);
    save_prompt(app, ctx);
    let Some(dialog) = app.dialog else {
        app.props_draft = None;
        return;
    };
    // Seed the editable Description fields from the document when the dialog opens.
    if let (Dialog::Properties(_), Some((_, id))) = (dialog, app.active_ids())
        && app.props_draft.as_ref().is_none_or(|(d, _)| *d != id)
        && let Some(doc) = app.session.get(id)
    {
        app.props_draft = Some((id, INFO_KEYS.map(|k| doc.info_value(k).unwrap_or_default())));
    }
    let mut apply = false;
    let t = Tokens::get(ctx);
    let mut close = false;
    let mut next = dialog;
    let modal = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
        ui.set_width(560.0);
        match dialog {
            Dialog::Properties(tab) => {
                ui.label(egui::RichText::new("Document Properties").font(theme::semibold(18.0)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    for (tb, label) in [
                        (PropsTab::Description, "Description"),
                        (PropsTab::Security, "Security"),
                        (PropsTab::Fonts, "Fonts"),
                        (PropsTab::Advanced, "Advanced"),
                    ] {
                        if widgets::mode_tab(ui, label, tab == tb).clicked() {
                            next = Dialog::Properties(tb);
                        }
                    }
                });
                ui.separator();
                let Some((_, id)) = app.active_ids() else { return };
                let Some(doc) = app.session.get(id) else { return };
                let i = &doc.info;
                let row = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.label(egui::RichText::new(k).color(t.text_muted));
                    ui.label(if v.is_empty() { egui::RichText::new("—").color(t.text_faint) } else { egui::RichText::new(v) });
                    ui.end_row();
                };
                egui::ScrollArea::vertical().max_height(460.0).auto_shrink([false, true]).show(ui, |ui| {
                    egui::Grid::new("props").num_columns(2).spacing([18.0, 8.0]).min_col_width(140.0).show(ui, |ui| match tab {
                        PropsTab::Description => {
                            row(ui, "File", doc.name.clone());
                            match app.props_draft.as_mut() {
                                Some((_, draft)) if doc.editable() => {
                                    for (k, v) in INFO_KEYS.iter().zip(draft.iter_mut()) {
                                        let l = ui.label(egui::RichText::new(*k).color(t.text_muted));
                                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY).id_salt(("info", *k))).labelled_by(l.id);
                                        ui.end_row();
                                    }
                                }
                                _ => {
                                    row(ui, "Title", i.title.clone().unwrap_or_default());
                                    row(ui, "Author", i.author.clone().unwrap_or_default());
                                    row(ui, "Subject", i.subject.clone().unwrap_or_default());
                                    row(ui, "Keywords", i.keywords.clone().unwrap_or_default());
                                }
                            }
                            row(ui, "Application", i.creator.clone().unwrap_or_default());
                            row(ui, "PDF producer", i.producer.clone().unwrap_or_default());
                        }
                        PropsTab::Security => {
                            row(ui, "Security method", if i.encrypted { "Password security".into() } else { "No security".into() });
                            for k in [
                                "Printing",
                                "Changing the document",
                                "Document assembly",
                                "Content copying",
                                "Commenting",
                                "Filling of form fields",
                                "Signing",
                            ] {
                                row(ui, k, if i.encrypted { "See permissions (M8)".into() } else { "Allowed".into() });
                            }
                        }
                        PropsTab::Fonts => {
                            if i.fonts.is_empty() {
                                row(ui, "Fonts", "No fonts are referenced by the pages.".into());
                            }
                            for f in &i.fonts {
                                let mut detail = f.kind.clone();
                                if let Some(e) = &f.encoding {
                                    detail.push_str(&format!(" · {e}"));
                                }
                                detail.push_str(if f.subset {
                                    " · Embedded subset"
                                } else if f.embedded {
                                    " · Embedded"
                                } else {
                                    " · Not embedded (substituted)"
                                });
                                row(ui, &f.name, detail);
                            }
                        }
                        PropsTab::Advanced => {
                            row(ui, "PDF version", i.pdf_version.replace("Pdf", "").replace('_', "."));
                            row(ui, "Location", doc.path.clone().unwrap_or_default());
                            row(ui, "File size", format!("{} ({} bytes)", human_size(i.file_size), i.file_size));
                            let p = &i.pages[0];
                            row(ui, "Page size", format!("{:.2} × {:.2} in", p.width / 72.0, p.height / 72.0));
                            row(ui, "Number of pages", i.pages.len().to_string());
                            row(ui, "Tagged PDF", yes(i.tagged));
                            row(ui, "Form fields", i.fields.len().to_string());
                            row(ui, "Comments", i.annotations.len().to_string());
                            row(ui, "Layers", i.layers.len().to_string());
                            row(ui, "Attachments", i.attachments.len().to_string());
                            row(ui, "JavaScript", yes(i.has_javascript));
                        }
                    })
                });
            }
            Dialog::Shortcuts => {
                ui.label(egui::RichText::new("Keyboard shortcuts").font(theme::semibold(18.0)));
                ui.add_space(8.0);
                egui::Grid::new("keys").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (k, v) in [
                        ("⌘O", "Open"),
                        ("⌘W", "Close file"),
                        ("⌘S / ⇧⌘S", "Save / Save as"),
                        ("⌘Z / ⇧⌘Z", "Undo / Redo"),
                        ("Delete", "Delete selected pages (Organize)"),
                        ("⌘D", "Document properties"),
                        ("⌘K", "Find tools and commands"),
                        ("⌘F", "Find text in the document"),
                        ("⌘G / ⇧⌘G", "Next / previous match"),
                        ("⌘C", "Copy selected text"),
                        ("Double-click", "Select a word"),
                        ("Esc", "Clear selection / close find"),
                        ("⌘1", "Actual size"),
                        ("⌘0", "Zoom to page level"),
                        ("⌘2", "Fit to width"),
                        ("⌘+ / ⌘−", "Zoom in / out (also pinch or ⌘-scroll)"),
                        ("Home / End", "First / last page"),
                        ("⌘← / ⌘→", "Previous / next page"),
                        ("⌃⌘H", "Read mode"),
                    ] {
                        ui.label(egui::RichText::new(k).font(egui::FontId::monospace(12.5)));
                        ui.label(v);
                        ui.end_row();
                    }
                });
            }
            Dialog::About => {
                ui.label(egui::RichText::new("PrintCraft").font(theme::semibold(20.0)));
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.label("A clean-room, open-source PDF application written in Rust. MIT OR Apache-2.0.");
                ui.label(
                    egui::RichText::new("Rendering: hayro (bootstrap) · UI: egui · Icons: Lucide (ISC) · Fonts: Inter, JetBrains Mono (OFL)")
                        .color(t.text_muted)
                        .small(),
                );
            }
        }
        ui.add_space(12.0);
        let changed = draft_changes(app).is_some_and(|c| !c.is_empty());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if changed {
                if widgets::pill_button(ui, "OK", true).clicked() {
                    apply = true;
                    close = true;
                }
                if widgets::pill_button(ui, "Cancel", false).clicked() {
                    close = true;
                }
            } else if widgets::pill_button(ui, "Close", true).clicked() {
                close = true;
            }
        });
    });
    if apply && let Some(edits) = draft_changes(app) {
        app.apply_edit(Edit::Batch { label: "Change document properties".into(), edits });
    }
    if modal.should_close() || close {
        app.dialog = None;
        app.props_draft = None;
    } else {
        app.dialog = Some(next);
    }
}

/// Info edits needed to make the document match the Description draft.
fn draft_changes(app: &PrintCraftApp) -> Option<Vec<Edit>> {
    let (id, draft) = app.props_draft.as_ref()?;
    let doc = app.session.get(*id)?;
    Some(
        INFO_KEYS
            .iter()
            .zip(draft.iter())
            .filter(|(k, v)| doc.info_value(k).unwrap_or_default().trim() != v.trim())
            .map(|(k, v)| Edit::SetInfo { key: (*k).to_string(), value: v.clone() })
            .collect(),
    )
}

/// "Save changes?" when closing a tab or quitting with unsaved edits.
fn save_prompt(app: &mut PrintCraftApp, ctx: &egui::Context) {
    let Some(req) = app.close_request else { return };
    let index = match req {
        CloseRequest::Tab(i) => Some(i),
        CloseRequest::Quit => app.first_dirty(),
    };
    let Some(name) = index.and_then(|i| app.views.get(i)).and_then(|v| app.session.get(v.id)).map(|d| d.name.clone()) else {
        // Nothing left to ask about (tab already gone or no dirty documents).
        app.resolve_close(ctx, Some(false));
        return;
    };
    let t = Tokens::get(ctx);
    let mut choice: Option<Option<bool>> = None;
    let modal = egui::Modal::new(egui::Id::new("save_prompt")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("save", 22.0, t.accent));
            ui.label(egui::RichText::new(format!("Save changes to “{name}” before closing?")).font(theme::semibold(16.0)));
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Your changes will be lost if you don't save them.").color(t.text_muted));
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::pill_button(ui, "Save", true).clicked() {
                choice = Some(Some(true));
            }
            if widgets::pill_button(ui, "Cancel", false).clicked() {
                choice = Some(None);
            }
            ui.add_space(24.0);
            if widgets::pill_button(ui, "Don't save", false).clicked() {
                choice = Some(Some(false));
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(None);
    }
    if let Some(c) = choice {
        app.resolve_close(ctx, c);
    }
}

fn yes(b: bool) -> String {
    if b { "Yes".into() } else { "No".into() }
}

/// Password prompt for encrypted documents (Acrobat: "Password" dialog on open).
fn password(app: &mut PrintCraftApp, ctx: &egui::Context) {
    let Some(prompt) = app.password_prompt.as_mut() else { return };
    let t = Tokens::get(ctx);
    let mut submit = false;
    let mut cancel = false;
    let modal = egui::Modal::new(egui::Id::new("password")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("lock", 22.0, t.accent));
            ui.label(egui::RichText::new("Password required").font(theme::semibold(17.0)));
        });
        ui.add_space(6.0);
        ui.label(format!("“{}” is protected. Enter a password to open it.", prompt.name));
        ui.add_space(8.0);
        let r = ui.add(egui::TextEdit::singleline(&mut prompt.input).password(true).hint_text("Password").desired_width(f32::INFINITY));
        r.request_focus();
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        if let Some(e) = &prompt.error {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(e).color(egui::Color32::from_rgb(0xD1, 0x3B, 0x3B)));
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::pill_button(ui, "Open", true).clicked() {
                submit = true;
            }
            if widgets::pill_button(ui, "Cancel", false).clicked() {
                cancel = true;
            }
        });
    });
    if submit {
        let pw = prompt.input.clone();
        app.submit_password(Some(pw));
    } else if cancel || modal.should_close() {
        app.submit_password(None);
    }
}
