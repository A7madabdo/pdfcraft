//! Modal dialogs: Document Properties, Keyboard Shortcuts, About.

use egui::{Align, Layout, vec2};

use crate::theme::{self, Tokens};
use crate::{Dialog, PrintCraftApp, PropsTab, panels::human_size, widgets};

pub fn show(app: &mut PrintCraftApp, ctx: &egui::Context) {
    let Some(dialog) = app.dialog else { return };
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
                    for (tb, label) in [(PropsTab::Description, "Description"), (PropsTab::Security, "Security"), (PropsTab::Advanced, "Advanced")] {
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
                egui::Grid::new("props").num_columns(2).spacing([18.0, 8.0]).min_col_width(140.0).show(ui, |ui| match tab {
                    PropsTab::Description => {
                        row(ui, "File", doc.name.clone());
                        row(ui, "Title", i.title.clone().unwrap_or_default());
                        row(ui, "Author", i.author.clone().unwrap_or_default());
                        row(ui, "Subject", i.subject.clone().unwrap_or_default());
                        row(ui, "Keywords", i.keywords.clone().unwrap_or_default());
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
                });
            }
            Dialog::Shortcuts => {
                ui.label(egui::RichText::new("Keyboard shortcuts").font(theme::semibold(18.0)));
                ui.add_space(8.0);
                egui::Grid::new("keys").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (k, v) in [
                        ("⌘O", "Open"),
                        ("⌘W", "Close file"),
                        ("⌘D", "Document properties"),
                        ("⌘K", "Find tools and commands"),
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
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::pill_button(ui, "Close", true).clicked() {
                close = true;
            }
        });
        let _ = vec2(0.0, 0.0);
    });
    if modal.should_close() || close {
        app.dialog = None;
    } else {
        app.dialog = Some(next);
    }
}

fn yes(b: bool) -> String {
    if b { "Yes".into() } else { "No".into() }
}
