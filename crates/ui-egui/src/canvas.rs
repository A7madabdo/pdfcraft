//! The document area: page layout, zoom, render scheduling, overlays (links, annotation hovers,
//! form-field highlights), and the organize-pages grid.
//!
//! Page images come from the engine's render pool as whole-page rasters at the current zoom;
//! stale rasters are shown stretched until the sharp one arrives (tiling comes in M3.3).

use std::collections::HashMap;

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2, pos2, vec2};
use printcraft_engine::DocId;
use printcraft_render::{DocInfo, LinkTarget, RenderPool, RenderRequest};

use crate::theme::{self, Tokens};
use crate::{PrintCraftApp, QuickTool, RightPanel, icons};

/// Logical pixels per PDF point at 100% (96 dpi, like browsers).
pub const PT: f32 = 96.0 / 72.0;
const GAP: f32 = 14.0;
const MARGIN: f32 = 28.0;
/// Horizontal gutter that keeps pages clear of the floating quick-action bar.
const SIDE: f32 = 70.0;
const THUMB_TAG: u64 = 1 << 63;
const THUMB_W: f32 = 132.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Width,
    Page,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageLayout {
    Continuous,
    TwoUp,
    Single,
}

struct PageTex {
    tag: u64,
    tex: TextureHandle,
}

pub struct DocView {
    pub id: DocId,
    pub zoom: f32,
    pub fit: Fit,
    pub layout: PageLayout,
    pub current: usize,
    pub organize: bool,
    pub highlight_fields: bool,
    pub page_input: String,
    pub notice_dismissed: bool,
    /// Pending navigation: page and fraction down the page to align with the viewport top.
    pub goto: Option<(usize, f32)>,
    /// Briefly outline an annotation after navigating to it from a panel.
    pub flash: Option<(usize, [f32; 4], f64)>,
    pages: HashMap<usize, PageTex>,
    thumbs: HashMap<usize, TextureHandle>,
    last_queue: Vec<RenderRequest>,
    viewport_w: f32,
    viewport_h: f32,
    page_count: usize,
}

impl DocView {
    pub fn new(id: DocId, info: &DocInfo) -> Self {
        Self {
            id,
            zoom: 1.0,
            fit: Fit::Width,
            layout: PageLayout::Continuous,
            current: 0,
            organize: false,
            highlight_fields: false,
            page_input: "1".into(),
            notice_dismissed: false,
            goto: None,
            flash: None,
            pages: HashMap::new(),
            thumbs: HashMap::new(),
            last_queue: Vec::new(),
            viewport_w: 800.0,
            viewport_h: 600.0,
            page_count: info.pages.len(),
        }
    }

    pub fn thumb(&self, page: usize) -> Option<&TextureHandle> {
        self.thumbs.get(&page)
    }

    pub fn go_to_page(&mut self, page: usize) {
        let page = page.min(self.page_count.saturating_sub(1));
        self.goto = Some((page, 0.0));
        self.current = page;
        self.page_input = (page + 1).to_string();
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let frac = 0.0;
        self.zoom = zoom.clamp(0.08, 64.0);
        self.fit = Fit::None;
        self.goto = Some((self.current, frac));
    }

    /// Standard zoom steps (as in Acrobat's zoom menu).
    pub fn zoom_step(&mut self, up: bool) {
        const STEPS: [f32; 17] = [0.1, 0.25, 0.33, 0.5, 0.66, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 32.0];
        let z = self.zoom;
        let next = if up { STEPS.iter().copied().find(|s| *s > z * 1.01) } else { STEPS.iter().rev().copied().find(|s| *s < z * 0.99) };
        if let Some(n) = next {
            self.set_zoom(n);
        }
    }

    /// Pull finished renders into textures.
    pub fn receive(&mut self, ctx: &egui::Context, pool: &RenderPool) {
        let mut got = false;
        while let Some(r) = pool.try_recv() {
            got = true;
            let img = egui::ColorImage::from_rgba_premultiplied([r.width as usize, r.height as usize], &r.rgba);
            let page = r.request.page;
            if r.request.tag & THUMB_TAG != 0 {
                let tex = ctx.load_texture(format!("thumb-{:?}-{page}", self.id), img, TextureOptions::LINEAR);
                self.thumbs.insert(page, tex);
            } else {
                let tex = ctx.load_texture(format!("page-{:?}-{page}", self.id), img, TextureOptions::LINEAR);
                self.pages.insert(page, PageTex { tag: r.request.tag, tex });
            }
        }
        if got {
            ctx.request_repaint();
        }
    }

    fn fit_zoom(&mut self, info: &DocInfo) {
        let max_w = info.pages.iter().map(|p| p.width).fold(1.0, f32::max);
        let avail_w = (self.viewport_w - 2.0 * SIDE).max(100.0);
        let per_row = if self.layout == PageLayout::TwoUp { 2.0 } else { 1.0 };
        match self.fit {
            Fit::Width => self.zoom = (avail_w - GAP * (per_row - 1.0)) / (max_w * PT * per_row),
            Fit::Page => {
                let p = &info.pages[self.current.min(info.pages.len() - 1)];
                let zw = (avail_w - GAP * (per_row - 1.0)) / (p.width * PT * per_row);
                let zh = (self.viewport_h - 2.0 * MARGIN) / (p.height * PT);
                self.zoom = zw.min(zh);
            }
            Fit::None => {}
        }
        self.zoom = self.zoom.clamp(0.08, 64.0);
    }

    /// Page rects in content coordinates (origin at the scroll content's top-left).
    fn layout(&self, info: &DocInfo, content_w: f32) -> Vec<Rect> {
        let s = self.zoom * PT;
        let mut rects = Vec::with_capacity(info.pages.len());
        let mut y = MARGIN;
        match self.layout {
            PageLayout::Continuous | PageLayout::Single => {
                for p in &info.pages {
                    let size = vec2(p.width * s, p.height * s);
                    rects.push(Rect::from_min_size(pos2(((content_w - size.x) / 2.0).max(SIDE), y), size));
                    y += size.y + GAP;
                }
            }
            PageLayout::TwoUp => {
                for pair in info.pages.chunks(2) {
                    let sizes: Vec<Vec2> = pair.iter().map(|p| vec2(p.width * s, p.height * s)).collect();
                    let row_w: f32 = sizes.iter().map(|v| v.x).sum::<f32>() + GAP * (sizes.len() as f32 - 1.0);
                    let row_h = sizes.iter().map(|v| v.y).fold(0.0, f32::max);
                    let mut x = ((content_w - row_w) / 2.0).max(SIDE);
                    for size in sizes {
                        rects.push(Rect::from_min_size(pos2(x, y + (row_h - size.y) / 2.0), size));
                        x += size.x + GAP;
                    }
                    y += row_h + GAP;
                }
            }
        }
        rects
    }

    fn render_scale(&self, ppp: f32) -> f32 {
        // Quantize so tiny fit-width changes don't trigger re-renders.
        ((self.zoom * PT * ppp) * 64.0).round() / 64.0
    }
}

/// Map a user-space rect on a page to screen space, honouring crop box and rotation.
pub fn page_to_screen(info: &DocInfo, page: usize, page_rect: Rect, r: [f32; 4]) -> Rect {
    let p = &info.pages[page];
    let [cx0, cy0, cx1, cy1] = p.crop;
    let (cw, ch) = ((cx1 - cx0).max(1.0), (cy1 - cy0).max(1.0));
    let map = |x: f32, y: f32| -> Pos2 {
        // Normalised coordinates in the unrotated crop box, y down.
        let (u, v) = ((x - cx0) / cw, (cy1 - y) / ch);
        let (u, v) = match p.rotation {
            90 => (1.0 - v, u),
            180 => (1.0 - u, 1.0 - v),
            270 => (v, 1.0 - u),
            _ => (u, v),
        };
        pos2(page_rect.left() + u * page_rect.width(), page_rect.top() + v * page_rect.height())
    };
    Rect::from_two_pos(map(r[0], r[1]), map(r[2], r[3]))
}

pub fn shortcuts(view: &mut DocView, ctx: &egui::Context) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
    let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
    if pressed(cmd(Key::Plus)) || pressed(cmd(Key::Equals)) {
        view.zoom_step(true);
    }
    if pressed(cmd(Key::Minus)) {
        view.zoom_step(false);
    }
    if pressed(cmd(Key::Num0)) {
        view.fit = Fit::Page;
        view.goto = Some((view.current, 0.0));
    }
    if pressed(cmd(Key::Num1)) {
        view.set_zoom(1.0);
    }
    if pressed(cmd(Key::Num2)) {
        view.fit = Fit::Width;
        view.goto = Some((view.current, 0.0));
    }
    let key = |k| ctx.input(|i| i.key_pressed(k));
    if key(Key::Home) {
        view.go_to_page(0);
    }
    if key(Key::End) {
        view.go_to_page(usize::MAX);
    }
    if view.layout == PageLayout::Single || ctx.input(|i| i.modifiers.command) {
        if key(Key::ArrowRight) || key(Key::PageDown) {
            view.go_to_page(view.current + 1);
        }
        if key(Key::ArrowLeft) || key(Key::PageUp) {
            view.go_to_page(view.current.saturating_sub(1));
        }
    }
}

pub fn document_area(app: &mut PrintCraftApp, index: usize, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.get(app.views[index].id) else { return };
    let info = &doc.info;
    if info.pages.is_empty() {
        ui.centered_and_justified(|ui| ui.label("This document has no pages."));
        return;
    }
    let want_thumbs = app.right == Some(RightPanel::Pages) || app.views[index].organize;
    let view = &mut app.views[index];
    notices(view, info, ui, &t);
    if view.organize {
        organize_grid(view, info, &doc.renderer, ui, &t);
        return;
    }

    let avail = ui.available_rect_before_wrap();
    view.viewport_w = avail.width();
    view.viewport_h = avail.height();
    view.fit_zoom(info);

    // Pinch / ctrl+scroll zoom, anchored on the current page.
    let zoom_delta = ui.input(|i| i.zoom_delta());
    if (zoom_delta - 1.0).abs() > 0.001 && ui.rect_contains_pointer(avail) {
        let z = view.zoom * zoom_delta;
        view.set_zoom(z);
    }

    let max_w = info.pages.iter().map(|p| p.width).fold(0.0, f32::max) * view.zoom * PT * if view.layout == PageLayout::TwoUp { 2.0 } else { 1.0 };
    let content_w = (max_w + 2.0 * SIDE).max(avail.width());
    let rects = view.layout(info, content_w);
    let visible_pages: Vec<usize> = match view.layout {
        PageLayout::Single => vec![view.current.min(rects.len() - 1)],
        _ => (0..rects.len()).collect(),
    };
    let (y_shift, content_h) = match view.layout {
        PageLayout::Single => {
            let r = rects[visible_pages[0]];
            (r.top() - MARGIN, r.height() + 2.0 * MARGIN)
        }
        _ => (0.0, rects.last().map(|r| r.bottom() + MARGIN).unwrap_or(0.0)),
    };

    let mut scroll = egui::ScrollArea::both().auto_shrink([false, false]).scroll_source(egui::scroll_area::ScrollSource {
        drag: if app.quick_tool == QuickTool::Hand { egui::scroll_area::DragScroll::Always } else { egui::scroll_area::DragScroll::OnTouch },
        ..Default::default()
    });
    if let Some((page, frac)) = view.goto.take() {
        let page = page.min(rects.len() - 1);
        let r = rects[page];
        scroll = scroll.vertical_scroll_offset((r.top() - y_shift - GAP + frac * r.height()).max(0.0));
    }
    let ppp = ui.ctx().pixels_per_point();
    let scale = view.render_scale(ppp);
    let tag = (scale * 1000.0) as u64;
    let hand = app.quick_tool == QuickTool::Hand;
    let mut hover_text: Option<(Pos2, String)> = None;
    let mut clicked_link: Option<LinkTarget> = None;

    let out = scroll.show_viewport(ui, |ui, viewport| {
        let (resp_rect, resp) = ui.allocate_exact_size(vec2(content_w, content_h), Sense::click());
        let origin = resp_rect.min - vec2(0.0, y_shift);
        let painter = ui.painter();
        let visible = viewport.translate(resp_rect.min.to_vec2());
        let mut wanted = Vec::new();
        let mut current = view.current;
        let mut best_overlap = -1.0f32;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        for &i in &visible_pages {
            let r = rects[i].translate(origin.to_vec2());
            if !r.intersects(visible.expand(400.0)) {
                continue;
            }
            let overlap = r.intersect(visible).height();
            if overlap > best_overlap {
                best_overlap = overlap;
                current = i;
            }
            // Shadow + paper.
            painter.rect_filled(r.translate(vec2(0.0, 2.0)).expand(1.5), CornerRadius::same(2), t.page_shadow);
            painter.rect_filled(r, CornerRadius::ZERO, Color32::WHITE);
            match view.pages.get(&i) {
                Some(p) => {
                    painter.image(p.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    if p.tag != tag {
                        wanted.push(i);
                    }
                }
                None => {
                    wanted.push(i);
                    painter.text(r.center(), Align2::CENTER_CENTER, "Rendering…", theme::regular(12.0), t.text_faint);
                }
            }
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);

            // Form-field highlight (Acrobat's "Highlight existing fields").
            if view.highlight_fields {
                for f in info.fields.iter().filter(|f| f.page == Some(i)) {
                    if let Some(fr) = f.rect {
                        let sr = page_to_screen(info, i, r, fr);
                        painter.rect_filled(sr, CornerRadius::same(1), Color32::from_rgba_unmultiplied(0x6E, 0x8E, 0xF5, 48));
                        painter.rect_stroke(
                            sr,
                            CornerRadius::same(1),
                            Stroke::new(1.0, Color32::from_rgb(0x6E, 0x8E, 0xF5)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
            // Link hover + click.
            if let Some(p) = pointer {
                for l in info.links.iter().filter(|l| l.page == i) {
                    let sr = page_to_screen(info, i, r, l.rect);
                    if sr.contains(p) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        let label = match &l.target {
                            LinkTarget::Page(n) => format!("Go to page {}", info.pages.get(*n).map(|p| p.label.as_str()).unwrap_or("?")),
                            LinkTarget::Uri(u) => u.clone(),
                            LinkTarget::Other(s) => format!("{s} action"),
                        };
                        hover_text = Some((p, label));
                        if resp.clicked() && !hand {
                            clicked_link = Some(l.target.clone());
                        }
                    }
                }
                // Annotation hover shows the comment, as Acrobat's popups do.
                for a in info.annotations.iter().filter(|a| a.page == i) {
                    let sr = page_to_screen(info, i, r, a.rect);
                    if sr.contains(p) && hover_text.is_none() {
                        let who = a.author.clone().unwrap_or_else(|| a.subtype.clone());
                        let body = a.contents.clone().unwrap_or_default();
                        hover_text = Some((p, if body.is_empty() { who } else { format!("{who}\n{body}") }));
                    }
                }
            }
            if let Some((fp, fr, t0)) = view.flash
                && fp == i
            {
                let now = ui.input(|inp| inp.time);
                let t0 = if t0 == 0.0 { now } else { t0 };
                view.flash = Some((fp, fr, t0));
                let age = (now - t0) as f32;
                if age < 1.6 {
                    let a = ((1.6 - age) / 1.6 * 255.0) as u8;
                    let sr = page_to_screen(info, i, r, fr).expand(4.0);
                    painter.rect_stroke(
                        sr,
                        CornerRadius::same(3),
                        Stroke::new(2.5, Color32::from_rgba_unmultiplied(0x1B, 0x63, 0xE0, a)),
                        egui::StrokeKind::Outside,
                    );
                    ui.ctx().request_repaint();
                } else {
                    view.flash = None;
                }
            }
        }
        if view.layout != PageLayout::Single {
            view.current = current;
            if !ui.memory(|m| m.has_focus(egui::Id::new("page-input"))) {
                view.page_input = (current + 1).to_string();
            }
        }
        wanted
    });

    // Schedule renders: visible pages first (nearest the current page), then thumbnails.
    let mut wanted = out.inner;
    let cur = view.current;
    wanted.sort_by_key(|&p| (p as isize - cur as isize).unsigned_abs());
    let mut queue: Vec<RenderRequest> = wanted.into_iter().map(|page| RenderRequest { page, scale, tag }).collect();
    if want_thumbs {
        let s = THUMB_W * ppp / info.pages.iter().map(|p| p.width).fold(1.0, f32::max);
        for page in 0..info.pages.len() {
            if !view.thumbs.contains_key(&page) {
                queue.push(RenderRequest { page, scale: s, tag: THUMB_TAG });
            }
        }
    }
    if queue != view.last_queue {
        doc.renderer.set_queue(queue.clone());
        view.last_queue = queue;
    }
    if !view.last_queue.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }

    if let Some((pos, text)) = hover_text {
        egui::Area::new(egui::Id::new("canvas-hover")).order(egui::Order::Tooltip).fixed_pos(pos + vec2(14.0, 16.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(320.0);
                ui.label(text);
            });
        });
    }
    match clicked_link {
        Some(LinkTarget::Page(p)) => view.go_to_page(p),
        Some(LinkTarget::Uri(u)) => ui.ctx().open_url(egui::OpenUrl::new_tab(u)),
        Some(LinkTarget::Other(s)) => app.notify(format!("{s} actions run in the JavaScript engine (M6)")),
        None => {}
    }
    quick_bar(app, avail, ui);
}

fn notices(view: &mut DocView, info: &DocInfo, ui: &mut egui::Ui, t: &Tokens) {
    if view.notice_dismissed {
        return;
    }
    let msg = if !info.fields.is_empty() {
        Some(("text-cursor-input", format!("This document contains {} interactive form fields.", info.fields.len()), true))
    } else if !info.warnings.is_empty() {
        Some(("triangle-alert", info.warnings[0].clone(), false))
    } else {
        None
    };
    let Some((icon, text, fields)) = msg else { return };
    egui::Frame::NONE.fill(t.accent_soft).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(icons::image(icon, 16.0, t.accent_text));
            ui.label(egui::RichText::new(text).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, "x", 22.0, false, "Dismiss").clicked() {
                    view.notice_dismissed = true;
                }
                if fields {
                    let label = if view.highlight_fields { "Hide field highlights" } else { "Highlight fields" };
                    if crate::widgets::pill_button(ui, label, view.highlight_fields).clicked() {
                        view.highlight_fields = !view.highlight_fields;
                    }
                }
            });
        });
    });
}

/// The floating quick-action bar at the left edge of the document area.
fn quick_bar(app: &mut PrintCraftApp, area: Rect, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let pos = area.left_top() + vec2(14.0, 14.0);
    egui::Area::new(egui::Id::new("quick-bar")).order(egui::Order::Middle).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(if t.dark() { 80 } else { 22 }) })
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.vertical(|ui| {
                    if icons::button(ui, "mouse-pointer-2", 32.0, app.quick_tool == QuickTool::Select, "Select (V)").clicked() {
                        app.quick_tool = QuickTool::Select;
                    }
                    if icons::button(ui, "hand", 32.0, app.quick_tool == QuickTool::Hand, "Hand (H)").clicked() {
                        app.quick_tool = QuickTool::Hand;
                    }
                    for (icon, tip, cmd) in [
                        ("message-square-text", "Add a comment (M5)", "comment.note"),
                        ("highlighter", "Highlight text (M5)", "comment.highlight"),
                        ("pencil", "Draw freehand (M5)", "comment.ink"),
                        ("pen-line", "Fill & Sign (M5)", "sign.fill.text"),
                    ] {
                        if icons::button(ui, icon, 32.0, false, tip).clicked() {
                            app.run_command(cmd);
                        }
                    }
                });
            });
    });
}

/// Organize pages: a thumbnail grid (Acrobat's Organize Pages view).
fn organize_grid(view: &mut DocView, info: &DocInfo, pool: &RenderPool, ui: &mut egui::Ui, t: &Tokens) {
    let ppp = ui.ctx().pixels_per_point();
    let cell = vec2(190.0, 250.0);
    let mut open_page = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(20.0);
        let cols = ((ui.available_width() - 40.0) / cell.x).floor().max(1.0) as usize;
        let rows = info.pages.len().div_ceil(cols);
        let left = (ui.available_width() - cols as f32 * cell.x) / 2.0;
        for row in 0..rows {
            let (row_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), cell.y), Sense::hover());
            for col in 0..cols {
                let i = row * cols + col;
                let Some(p) = info.pages.get(i) else { break };
                let c = Rect::from_min_size(pos2(row_rect.left() + left + col as f32 * cell.x, row_rect.top()), cell);
                let resp = ui.interact(c, ui.id().with(("org", i)), Sense::click());
                let s = (cell.x - 44.0) / p.width.max(1.0);
                let size = vec2(p.width * s, p.height * s).min(vec2(cell.x - 44.0, cell.y - 56.0));
                let pr = Rect::from_center_size(pos2(c.center().x, c.top() + 16.0 + size.y / 2.0), size);
                let selected = i == view.current;
                if selected || resp.hovered() {
                    ui.painter().rect_filled(c.shrink(6.0), CornerRadius::same(8), if selected { t.accent_soft } else { t.hover });
                }
                ui.painter().rect_filled(pr.translate(vec2(0.0, 1.5)), CornerRadius::same(1), t.page_shadow);
                ui.painter().rect_filled(pr, CornerRadius::ZERO, Color32::WHITE);
                if let Some(tex) = view.thumbs.get(&i) {
                    ui.painter().image(tex.id(), pr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                ui.painter().rect_stroke(pr, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
                ui.painter().text(pos2(c.center().x, pr.bottom() + 16.0), Align2::CENTER_CENTER, &p.label, theme::medium(12.0), t.text_muted);
                if resp.clicked() {
                    view.current = i;
                }
                if resp.double_clicked() {
                    open_page = Some(i);
                }
            }
        }
    });
    let s = THUMB_W * ppp / info.pages.iter().map(|p| p.width).fold(1.0, f32::max);
    let queue: Vec<RenderRequest> =
        (0..info.pages.len()).filter(|p| !view.thumbs.contains_key(p)).map(|page| RenderRequest { page, scale: s, tag: THUMB_TAG }).collect();
    if queue != view.last_queue {
        pool.set_queue(queue.clone());
        view.last_queue = queue;
    }
    if let Some(p) = open_page {
        view.organize = false;
        view.go_to_page(p);
    }
}
