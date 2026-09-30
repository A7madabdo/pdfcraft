//! Left tool panel (All tools + tool sub-panels, generated from the engine catalogue) and the
//! right-hand panels (Comments, Bookmarks, Pages, Fields, Layers, Attachments).

use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, pos2, vec2};
use printcraft_engine::catalog::{self, Availability, TOOL_GROUPS, ToolGroup};
use printcraft_render::{Annotation, DocInfo, FieldKind, OutlineItem};

use crate::theme::{self, Tokens};
use crate::{LeftPanel, PrintCraftApp, RightPanel, icons, widgets};

const COLLAPSED_TOOLS: usize = 14;

fn hue(g: &ToolGroup) -> Color32 {
    Color32::from_rgb(g.hue[0], g.hue[1], g.hue[2])
}

pub fn left_panel(app: &mut PrintCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::left("tool_panel")
        .resizable(false)
        .exact_size(272.0)
        .frame(
            egui::Frame::NONE
                .fill(t.panel)
                .inner_margin(egui::Margin { left: 14, right: 10, top: 12, bottom: 10 })
                .stroke(Stroke::new(1.0, t.divider)),
        )
        .show(ui, |ui| match app.left {
            LeftPanel::AllTools => all_tools(app, ui, &t),
            LeftPanel::Tool(id) => match catalog::group(id) {
                Some(g) => tool_detail(app, ui, &t, g),
                None => app.left = LeftPanel::AllTools,
            },
        });
}

fn panel_header(ui: &mut egui::Ui, t: &Tokens, title: &str, back: bool) -> (bool, bool) {
    let mut go_back = false;
    let mut close = false;
    ui.horizontal(|ui| {
        if back && icons::button(ui, "chevron-left", 26.0, false, "Back to all tools").clicked() {
            go_back = true;
        }
        ui.label(egui::RichText::new(title).font(theme::semibold(15.5)).color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icons::button(ui, "x", 26.0, false, "Close panel").clicked() {
                close = true;
            }
        });
    });
    ui.add_space(6.0);
    (go_back, close)
}

fn all_tools(app: &mut PrintCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let (_, close) = panel_header(ui, t, "All tools", false);
    if close {
        app.left_open = false;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let shown = if app.all_tools_expanded { TOOL_GROUPS.len() } else { COLLAPSED_TOOLS.min(TOOL_GROUPS.len()) };
        for g in &TOOL_GROUPS[..shown] {
            if tool_row(ui, t, g).clicked() {
                app.left = LeftPanel::Tool(g.id);
            }
        }
        ui.add_space(4.0);
        let more = if app.all_tools_expanded { "View less" } else { "View more" };
        if ui.add(egui::Label::new(egui::RichText::new(more).color(t.accent_text).font(theme::medium(13.0))).sense(Sense::click())).clicked() {
            app.all_tools_expanded = !app.all_tools_expanded;
        }
    });
}

fn tool_row(ui: &mut egui::Ui, t: &Tokens, g: &ToolGroup) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    icons::paint(ui, Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(20.0, 20.0)), g.icon, 19.0, hue(g));
    ui.painter().text(rect.left_center() + vec2(36.0, 0.0), Align2::LEFT_CENTER, g.label, theme::regular(13.5), t.text);
    let (chip, fill) = match (g.badge, g.availability) {
        (Some(b), _) => (Some(b.to_string()), t.badge_new),
        (None, Availability::Planned(m)) => (Some(m.to_string()), t.pressed),
        _ => (None, t.pressed),
    };
    if let Some(c) = chip {
        let font = theme::semibold(9.5);
        let fg = if fill == t.badge_new { Color32::WHITE } else { t.text_muted };
        let w = ui.fonts_mut(|f| f.layout_no_wrap(c.clone(), font.clone(), fg).size().x);
        let r = Rect::from_center_size(rect.right_center() - vec2(w / 2.0 + 10.0, 0.0), vec2(w + 10.0, 16.0));
        ui.painter().rect_filled(r, CornerRadius::same(4), fill);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, c, font, fg);
    }
    let tip = match g.availability {
        Availability::Ready => "Available".to_string(),
        Availability::Planned(m) => format!("Planned for milestone {m} — open to see what it will include"),
        Availability::Provider => "Optional: needs an AI provider you configure".into(),
    };
    resp.on_hover_text(tip)
}

fn tool_detail(app: &mut PrintCraftApp, ui: &mut egui::Ui, t: &Tokens, g: &'static ToolGroup) {
    let (back, close) = panel_header(ui, t, g.label, true);
    if back {
        app.left = LeftPanel::AllTools;
        app.mode = crate::Mode::AllTools;
    }
    if close {
        app.left_open = false;
    }
    if let Availability::Planned(m) = g.availability {
        egui::Frame::NONE.fill(t.accent_soft).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!("Coming in milestone {m}. Items marked Ready work today.")).color(t.text).font(theme::regular(12.0)),
            );
        });
    }
    let mut run = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for s in g.sections {
            widgets::section_title(ui, s.title);
            for item in s.items {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                let ready = item.availability == Availability::Ready;
                if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
                }
                let fg = if ready { t.text } else { t.text_muted };
                icons::paint(
                    ui,
                    Rect::from_min_size(rect.min + vec2(6.0, 8.0), vec2(18.0, 18.0)),
                    item.icon,
                    17.0,
                    if ready { hue(g) } else { t.text_faint },
                );
                ui.painter().text(rect.left_center() + vec2(34.0, 0.0), Align2::LEFT_CENTER, item.label, theme::regular(13.0), fg);
                let (chip, fill, cfg) = match item.availability {
                    Availability::Ready => ("Ready", Color32::from_rgb(0xDD, 0xF3, 0xE4), Color32::from_rgb(0x1E, 0x7B, 0x43)),
                    Availability::Planned(m) => (m, t.pressed, t.text_muted),
                    Availability::Provider => ("AI", t.pressed, t.text_muted),
                };
                let font = theme::semibold(9.5);
                let w = ui.fonts_mut(|f| f.layout_no_wrap(chip.to_string(), font.clone(), cfg).size().x);
                let r = Rect::from_center_size(rect.right_center() - vec2(w / 2.0 + 10.0, 0.0), vec2(w + 10.0, 16.0));
                ui.painter().rect_filled(r, CornerRadius::same(4), fill);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, chip, font, cfg);
                if resp.on_hover_text(item.command).clicked() {
                    run = Some(item.command);
                }
            }
        }
    });
    if let Some(cmd) = run {
        app.run_command(cmd);
    }
}

// ───────────────────────────────────────────────────────────────────────────── right panels

enum Nav {
    Page(usize),
    Flash(usize, [f32; 4]),
}

pub fn right_panel(app: &mut PrintCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((index, id)) = app.active_ids() else { return };
    let Some(panel) = app.right else { return };
    let mut nav: Option<Nav> = None;
    let mut close = false;
    let mut layer_toast = false;
    {
        let Some(doc) = app.session.get(id) else { return };
        let info = &doc.info;
        let view = &app.views[index];
        egui::Panel::right("right_panel")
            .resizable(true)
            .default_size(330.0)
            .size_range(260.0..=520.0)
            .frame(
                egui::Frame::NONE
                    .fill(t.panel)
                    .inner_margin(egui::Margin { left: 14, right: 12, top: 12, bottom: 8 })
                    .stroke(Stroke::new(1.0, t.divider)),
            )
            .show(ui, |ui| {
                let (title, count) = match panel {
                    RightPanel::Comments => ("Comments", Some(info.annotations.len())),
                    RightPanel::Bookmarks => ("Bookmarks", None),
                    RightPanel::Pages => ("Pages", Some(info.pages.len())),
                    RightPanel::Fields => ("Fields", Some(info.fields.len())),
                    RightPanel::Layers => ("Layers", Some(info.layers.len())),
                    RightPanel::Attachments => ("Attachments", Some(info.attachments.len())),
                };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).font(theme::semibold(15.5)));
                    if let Some(c) = count {
                        ui.label(egui::RichText::new(c.to_string()).font(theme::medium(13.0)).color(t.text_faint));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, "x", 26.0, false, "Close").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match panel {
                    RightPanel::Comments => comments(ui, &t, info, &mut nav),
                    RightPanel::Bookmarks => {
                        if info.outline.is_empty() {
                            empty(ui, &t, "bookmark", "This document has no bookmarks.");
                        }
                        for item in &info.outline {
                            outline_item(ui, &t, info, item, 0, &mut nav);
                        }
                    }
                    RightPanel::Pages => pages(ui, &t, info, view, &mut nav),
                    RightPanel::Fields => fields(ui, &t, info, &mut nav),
                    RightPanel::Layers => {
                        if info.layers.is_empty() {
                            empty(ui, &t, "layers", "This document has no layers.");
                        }
                        for l in &info.layers {
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
                            }
                            icons::paint(
                                ui,
                                Rect::from_min_size(rect.min + vec2(4.0, 7.0), vec2(18.0, 18.0)),
                                if l.visible { "eye" } else { "eye-off" },
                                16.0,
                                t.icon,
                            );
                            ui.painter().text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, &l.name, theme::regular(13.0), t.text);
                            if resp.clicked() {
                                layer_toast = true;
                            }
                        }
                    }
                    RightPanel::Attachments => {
                        if info.attachments.is_empty() {
                            empty(ui, &t, "paperclip", "This document has no attachments.");
                        }
                        for a in &info.attachments {
                            ui.horizontal(|ui| {
                                ui.add(icons::image("paperclip", 16.0, t.icon));
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&a.name).font(theme::medium(13.0)));
                                    let mut meta = a.size.map(human_size).unwrap_or_default();
                                    if let Some(d) = &a.description {
                                        meta = format!("{meta}  ·  {d}");
                                    }
                                    ui.label(egui::RichText::new(meta).color(t.text_faint).small());
                                });
                            });
                            ui.add_space(6.0);
                        }
                    }
                });
            });
    }
    if close {
        app.right = None;
    }
    if layer_toast {
        app.notify("Toggling layer visibility re-renders with optional content states — ships with the M2 renderer device");
    }
    match nav {
        Some(Nav::Page(p)) => app.views[index].go_to_page(p),
        Some(Nav::Flash(p, r)) => {
            let v = &mut app.views[index];
            v.go_to_page(p);
            v.flash = Some((p, r, 0.0));
        }
        None => {}
    }
}

fn empty(ui: &mut egui::Ui, t: &Tokens, icon: &str, text: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.add(icons::image(icon, 36.0, t.text_faint));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(text).color(t.text_muted));
    });
}

fn subtype_icon(s: &str) -> &'static str {
    match s {
        "Text" => "sticky-note",
        "Highlight" => "highlighter",
        "Underline" => "underline",
        "StrikeOut" => "strikethrough",
        "Squiggly" => "spline",
        "FreeText" => "type",
        "Ink" => "pencil",
        "Square" => "square",
        "Circle" => "circle",
        "Line" => "arrow-up-right",
        "Polygon" | "PolyLine" => "pen-tool",
        "Stamp" => "stamp",
        "FileAttachment" => "paperclip",
        "Caret" => "text-select",
        "Redact" => "rectangle-horizontal",
        _ => "message-square-text",
    }
}

fn comments(ui: &mut egui::Ui, t: &Tokens, info: &DocInfo, nav: &mut Option<Nav>) {
    if info.annotations.is_empty() {
        empty(ui, t, "message-square-text", "No comments yet.");
        return;
    }
    let mut page = usize::MAX;
    let roots: Vec<&Annotation> = info.annotations.iter().filter(|a| a.in_reply_to.is_none()).collect();
    for a in roots {
        if a.page != page {
            page = a.page;
            let n = info.annotations.iter().filter(|x| x.page == page && x.in_reply_to.is_none()).count();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Page {}", info.pages[page].label)).font(theme::semibold(12.5)).color(t.text_muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(egui::RichText::new(n.to_string()).color(t.text_faint).small()));
            });
        }
        let replies: Vec<&Annotation> = match &a.name {
            Some(nm) => info.annotations.iter().filter(|r| r.in_reply_to.as_deref() == Some(nm.as_str())).collect(),
            None => Vec::new(),
        };
        if comment_card(ui, t, a, &replies).clicked() {
            *nav = Some(Nav::Flash(a.page, a.rect));
        }
    }
}

fn comment_card(ui: &mut egui::Ui, t: &Tokens, a: &Annotation, replies: &[&Annotation]) -> egui::Response {
    let accent = a.color.map(|c| Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)).unwrap_or(t.accent);
    let resp = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.divider))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 13.0, accent.gamma_multiply(0.22));
                icons::paint(ui, r, subtype_icon(&a.subtype), 14.0, accent.gamma_multiply(1.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(egui::RichText::new(a.author.as_deref().unwrap_or("Unknown author")).font(theme::semibold(13.0)));
                    let meta = format!("{}{}", a.subtype, a.modified.as_deref().map(|m| format!("  ·  {m}")).unwrap_or_default());
                    ui.label(egui::RichText::new(meta).color(t.text_faint).font(theme::regular(11.0)));
                });
            });
            if let Some(c) = &a.contents {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(c).color(t.text));
            }
            for r in replies {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let (bar, _) = ui.allocate_exact_size(vec2(2.0, 30.0), Sense::hover());
                    ui.painter().rect_filled(bar, CornerRadius::same(1), t.divider);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(egui::RichText::new(r.author.as_deref().unwrap_or("Reply")).font(theme::semibold(12.0)));
                        if let Some(c) = &r.contents {
                            ui.label(egui::RichText::new(c).color(t.text_muted).font(theme::regular(12.0)));
                        }
                    });
                });
            }
        })
        .response;
    ui.add_space(6.0);
    ui.interact(resp.rect, ui.id().with(("comment", a.page, a.rect[0].to_bits(), a.rect[1].to_bits())), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn outline_item(ui: &mut egui::Ui, t: &Tokens, info: &DocInfo, item: &OutlineItem, depth: usize, nav: &mut Option<Nav>) {
    let indent = depth as f32 * 16.0;
    let id = ui.id().with(("outline", depth, item.title.as_str(), item.page));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(item.open || depth == 0);
    let font = if depth == 0 { theme::medium(13.0) } else { theme::regular(13.0) };
    let wrap_w = (ui.available_width() - indent - 20.0 - 36.0).max(60.0);
    let galley = ui.fonts_mut(|f| f.layout(item.title.clone(), font, t.text, wrap_w));
    let h = (galley.size().y + 12.0).max(28.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    let x0 = rect.left() + indent;
    if !item.children.is_empty() {
        let tri = Rect::from_min_size(pos2(x0, rect.top() + 6.0), vec2(16.0, 16.0));
        icons::paint(ui, tri, if open { "chevron-down" } else { "chevron-right" }, 14.0, t.text_muted);
        if ui.interact(tri, id.with("tri"), Sense::click()).clicked() {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
    }
    ui.painter().galley(pos2(x0 + 20.0, rect.top() + 6.0), galley, t.text);
    if let Some(p) = item.page {
        ui.painter().text(rect.right_top() + vec2(-6.0, 14.0), Align2::RIGHT_CENTER, &info.pages[p].label, theme::regular(11.0), t.text_faint);
    }
    if resp.clicked()
        && let Some(p) = item.page
    {
        *nav = Some(Nav::Page(p));
    }
    if open {
        for c in &item.children {
            outline_item(ui, t, info, c, depth + 1, nav);
        }
    }
}

fn pages(ui: &mut egui::Ui, t: &Tokens, info: &DocInfo, view: &crate::DocView, nav: &mut Option<Nav>) {
    let w = (ui.available_width() - 40.0).min(150.0);
    for (i, p) in info.pages.iter().enumerate() {
        ui.vertical_centered(|ui| {
            let h = w * p.height / p.width.max(1.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w + 16.0, h + 16.0), Sense::click());
            let selected = i == view.current;
            if selected {
                ui.painter().rect_filled(rect, CornerRadius::same(8), t.accent_soft);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(8), t.hover);
            }
            let pr = rect.shrink(8.0);
            ui.painter().rect_filled(pr, CornerRadius::ZERO, Color32::WHITE);
            if let Some(tex) = view.thumb(i) {
                ui.painter().image(tex.id(), pr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            ui.painter().rect_stroke(
                pr,
                CornerRadius::ZERO,
                Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.border }),
                egui::StrokeKind::Outside,
            );
            ui.label(egui::RichText::new(&p.label).font(theme::medium(12.0)).color(if selected { t.accent_text } else { t.text_muted }));
            if resp.clicked() {
                *nav = Some(Nav::Page(i));
            }
        });
        ui.add_space(4.0);
    }
}

fn fields(ui: &mut egui::Ui, t: &Tokens, info: &DocInfo, nav: &mut Option<Nav>) {
    if info.fields.is_empty() {
        empty(ui, t, "text-cursor-input", "This document has no form fields.");
        return;
    }
    let mut pages: Vec<Option<usize>> = info.fields.iter().map(|f| f.page).collect();
    pages.sort();
    pages.dedup();
    for p in pages {
        let label = p.map(|p| format!("Page {}", info.pages[p].label)).unwrap_or_else(|| "Unplaced".into());
        ui.add_space(4.0);
        ui.label(egui::RichText::new(label).font(theme::semibold(12.5)).color(t.text_muted));
        for f in info.fields.iter().filter(|f| f.page == p) {
            let icon = match f.kind {
                FieldKind::Text => "text-cursor-input",
                FieldKind::CheckBox => "check-circle-2",
                FieldKind::Radio => "circle",
                FieldKind::PushButton => "square",
                FieldKind::Combo => "chevron-down",
                FieldKind::List => "list",
                FieldKind::Signature => "signature",
                FieldKind::Unknown => "square",
            };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
            }
            icons::paint(ui, Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), icon, 15.0, Color32::from_rgb(0x8E, 0x4E, 0xE6));
            ui.painter().text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, &f.name, theme::regular(13.0), t.text);
            if let Some(v) = &f.value {
                let v: String = if v.chars().count() > 18 { format!("{}…", v.chars().take(17).collect::<String>()) } else { v.clone() };
                ui.painter().text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, v, theme::regular(11.5), t.text_faint);
            }
            let mut tip = format!("{:?} field", f.kind);
            if let Some(tt) = &f.tooltip {
                tip.push_str(&format!(" — {tt}"));
            }
            if f.has_actions {
                tip.push_str("\nHas JavaScript actions (run in M6)");
            }
            if resp.on_hover_text(tip).clicked()
                && let (Some(p), Some(r)) = (f.page, f.rect)
            {
                *nav = Some(Nav::Flash(p, r));
            }
        }
    }
}

pub fn human_size(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} bytes"),
    }
}
