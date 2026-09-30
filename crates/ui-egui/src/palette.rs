//! ⌘K command palette over the tool catalogue (fuzzy-ish substring match on tool and item names).

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, vec2};
use printcraft_engine::catalog::{Availability, TOOL_GROUPS};

use crate::theme::{self, Tokens};
use crate::{LeftPanel, PrintCraftApp, icons};

struct Hit {
    group: &'static str,
    label: String,
    detail: &'static str,
    icon: &'static str,
    command: Option<&'static str>,
    ready: bool,
}

fn score(hay: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let h = hay.to_lowercase();
    if let Some(p) = h.find(needle) {
        return Some(p);
    }
    // Subsequence match as a fallback.
    let mut it = h.chars();
    needle.chars().all(|c| it.any(|x| x == c)).then_some(100)
}

pub fn show(app: &mut PrintCraftApp, ctx: &egui::Context) {
    if !app.palette_open {
        return;
    }
    let t = Tokens::get(ctx);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.palette_open = false;
        return;
    }
    let q = app.palette_query.trim().to_lowercase();
    let mut hits: Vec<(usize, Hit)> = Vec::new();
    for g in TOOL_GROUPS {
        if let Some(s) = score(g.label, &q) {
            hits.push((
                s,
                Hit {
                    group: g.id,
                    label: g.label.to_string(),
                    detail: "Tool",
                    icon: g.icon,
                    command: None,
                    ready: g.availability == Availability::Ready,
                },
            ));
        }
        for sec in g.sections {
            for i in sec.items {
                if let Some(s) = score(i.label, &q).or_else(|| score(i.command, &q).map(|s| s + 50)) {
                    hits.push((
                        s + 1,
                        Hit {
                            group: g.id,
                            label: i.label.to_string(),
                            detail: g.label,
                            icon: i.icon,
                            command: Some(i.command),
                            ready: i.availability == Availability::Ready,
                        },
                    ));
                }
            }
        }
    }
    hits.sort_by_key(|(s, h)| (*s, !h.ready));
    hits.truncate(12);

    let screen = ctx.content_rect();
    let mut chosen: Option<(Option<&'static str>, &'static str)> = None;
    egui::Area::new(egui::Id::new("palette"))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(screen.center().x, screen.top() + 96.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(560.0);
                ui.horizontal(|ui| {
                    ui.add(icons::image("search", 18.0, t.text_muted));
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut app.palette_query)
                            .hint_text("Search tools and commands…")
                            .frame(egui::Frame::NONE)
                            .font(theme::regular(15.0))
                            .desired_width(f32::INFINITY),
                    );
                    r.request_focus();
                    if r.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && let Some((_, h)) = hits.first()
                    {
                        chosen = Some((h.command, h.group));
                    }
                });
                ui.separator();
                for (_, h) in &hits {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
                    }
                    icons::paint(ui, Rect::from_min_size(rect.min + vec2(8.0, 9.0), vec2(18.0, 18.0)), h.icon, 17.0, t.icon);
                    ui.painter().text(rect.left_center() + vec2(36.0, 0.0), Align2::LEFT_CENTER, &h.label, theme::regular(13.5), t.text);
                    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, h.detail, theme::regular(12.0), t.text_faint);
                    if resp.clicked() {
                        chosen = Some((h.command, h.group));
                    }
                }
                if hits.is_empty() {
                    ui.label(egui::RichText::new("No matching tools").color(t.text_muted));
                }
                ui.add_space(2.0);
                let _ = Stroke::NONE;
            });
        });
    if let Some((command, group)) = chosen {
        app.palette_open = false;
        app.palette_query.clear();
        app.left_open = true;
        app.left = LeftPanel::Tool(group);
        if let Some(c) = command {
            app.run_command(c);
        }
    }
}
