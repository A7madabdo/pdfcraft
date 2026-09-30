//! Document inspection: everything panels need that is not pixels.
//!
//! Page geometry comes from hayro (which resolves inheritance and rotation); the rest comes from
//! lopdf (bootstrap, replaced by `printcraft-model` in M2). Inspection is *tolerant*: when lopdf
//! cannot load a file that hayro can render, panels are simply empty and `warnings` says why.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use hayro::hayro_syntax::Pdf;
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::OpenError;

#[derive(Clone, Debug, Default)]
pub struct DocInfo {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub pdf_version: String,
    pub file_size: usize,
    pub encrypted: bool,
    pub tagged: bool,
    pub has_javascript: bool,
    pub pages: Vec<PageInfo>,
    pub outline: Vec<OutlineItem>,
    pub annotations: Vec<Annotation>,
    pub fields: Vec<Field>,
    pub links: Vec<Link>,
    pub layers: Vec<Layer>,
    pub attachments: Vec<Attachment>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PageInfo {
    /// Displayed size in points, after `/Rotate` and `/UserUnit`.
    pub width: f32,
    pub height: f32,
    /// Page label (`/PageLabels`), falling back to the 1-based page number.
    pub label: String,
    /// Effective crop box in user space [x0, y0, x1, y1] (the visible region).
    pub crop: [f32; 4],
    /// Clockwise page rotation in degrees (0, 90, 180, 270).
    pub rotation: u16,
}

/// A clickable link annotation.
#[derive(Clone, Debug)]
pub struct Link {
    pub page: usize,
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Page(usize),
    Uri(String),
    Other(String),
}

#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<usize>,
    pub children: Vec<OutlineItem>,
    pub open: bool,
}

#[derive(Clone, Debug)]
pub struct Annotation {
    pub page: usize,
    pub subtype: String,
    pub author: Option<String>,
    pub contents: Option<String>,
    pub modified: Option<String>,
    /// `/NM` of this annotation, and of the one it replies to (`/IRT`), for threading.
    pub name: Option<String>,
    pub in_reply_to: Option<String>,
    /// Rect in PDF user space: [x0, y0, x1, y1].
    pub rect: [f32; 4],
    pub color: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    CheckBox,
    Radio,
    PushButton,
    Combo,
    List,
    Signature,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub kind: FieldKind,
    pub value: Option<String>,
    pub page: Option<usize>,
    pub tooltip: Option<String>,
    pub has_actions: bool,
    /// Rect of the first widget, in user space.
    pub rect: Option<[f32; 4]>,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub description: Option<String>,
    pub size: Option<usize>,
}

/// Inspect a document. Fails only if the renderer itself cannot open the file.
pub fn inspect(bytes: Arc<Vec<u8>>) -> Result<DocInfo, OpenError> {
    let pdf = Pdf::new(bytes.clone()).map_err(|e| match format!("{e:?}") {
        s if s.to_lowercase().contains("encrypt") || s.to_lowercase().contains("password") => OpenError::NeedsPassword,
        s => OpenError::Invalid(s),
    })?;
    let mut info = DocInfo { file_size: bytes.len(), pdf_version: format!("{:?}", pdf.version()), ..Default::default() };
    for (i, page) in pdf.pages().iter().enumerate() {
        let (w, h) = page.render_dimensions();
        let c = page.intersected_crop_box();
        let rotation = match page.rotation() {
            hayro::hayro_syntax::page::Rotation::None => 0,
            hayro::hayro_syntax::page::Rotation::Horizontal => 90,
            hayro::hayro_syntax::page::Rotation::Flipped => 180,
            hayro::hayro_syntax::page::Rotation::FlippedHorizontal => 270,
        };
        let crop = [c.x0 as f32, c.y0 as f32, c.x1 as f32, c.y1 as f32];
        info.pages.push(PageInfo { width: w, height: h, label: (i + 1).to_string(), crop, rotation });
    }
    match Document::load_mem(&bytes) {
        Ok(doc) => Inspector::new(&doc).fill(&mut info),
        Err(e) => info.warnings.push(format!("structure inspection unavailable: {e}")),
    }
    Ok(info)
}

struct Inspector<'a> {
    doc: &'a Document,
    page_index: HashMap<ObjectId, usize>,
}

impl<'a> Inspector<'a> {
    fn new(doc: &'a Document) -> Self {
        let page_index = doc.get_pages().into_iter().map(|(n, id)| (id, n as usize - 1)).collect();
        Self { doc, page_index }
    }

    fn fill(&self, info: &mut DocInfo) {
        info.encrypted = self.doc.is_encrypted() || self.doc.trailer.get(b"Encrypt").is_ok();
        if let Some(d) = self.doc.trailer.get(b"Info").ok().and_then(|o| self.dict(o)) {
            info.title = self.text(d, b"Title");
            info.author = self.text(d, b"Author");
            info.subject = self.text(d, b"Subject");
            info.keywords = self.text(d, b"Keywords");
            info.creator = self.text(d, b"Creator");
            info.producer = self.text(d, b"Producer");
        }
        let Ok(catalog) = self.doc.catalog() else { return };
        info.tagged = catalog
            .get(b"MarkInfo")
            .ok()
            .and_then(|o| self.dict(o))
            .and_then(|m| m.get(b"Marked").ok())
            .and_then(|o| o.as_bool().ok())
            .unwrap_or(false);
        self.page_labels(catalog, &mut info.pages);
        if let Some(first) = catalog.get(b"Outlines").ok().and_then(|o| self.dict(o)).and_then(|d| d.get(b"First").ok()) {
            let mut seen = HashSet::new();
            info.outline = self.outline_siblings(first, &mut seen, 0);
        }
        self.annotations(info);
        if let Some(form) = catalog.get(b"AcroForm").ok().and_then(|o| self.dict(o))
            && let Ok(fields) = form.get(b"Fields").and_then(|o| self.resolve(o).as_array())
        {
            let mut seen = HashSet::new();
            for f in fields {
                self.field(f, None, &mut info.fields, &mut seen, 0);
            }
        }
        info.has_javascript |= info.fields.iter().any(|f| f.has_actions);
        self.layers(catalog, &mut info.layers);
        if let Some(names) = catalog.get(b"Names").ok().and_then(|o| self.dict(o)) {
            if let Some(ef) = names.get(b"EmbeddedFiles").ok().and_then(|o| self.dict(o)) {
                let mut seen = HashSet::new();
                for (name, spec) in self.name_tree(ef, &mut seen, 0) {
                    info.attachments.push(self.attachment(name, spec));
                }
            }
            info.has_javascript |= names.get(b"JavaScript").is_ok();
        }
    }

    // ── object helpers ──────────────────────────────────────────────────────────────────────

    fn resolve(&self, o: &'a Object) -> &'a Object {
        self.doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
    }

    fn dict(&self, o: &'a Object) -> Option<&'a Dictionary> {
        match self.resolve(o) {
            Object::Dictionary(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    fn text(&self, d: &Dictionary, key: &[u8]) -> Option<String> {
        let o = self.resolve(d.get(key).ok()?);
        let s = match o {
            Object::String(..) => lopdf::decode_text_string(o).ok()?,
            Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
            _ => return None,
        };
        let s = s.trim_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    fn name(&self, d: &Dictionary, key: &[u8]) -> Option<String> {
        self.resolve(d.get(key).ok()?).as_name().ok().map(|n| String::from_utf8_lossy(n).into_owned())
    }

    fn page_of(&self, o: &Object) -> Option<usize> {
        match o {
            Object::Reference(id) => self.page_index.get(id).copied(),
            // Some producers write page *numbers* in remote-style destinations.
            Object::Integer(n) => usize::try_from(*n).ok(),
            _ => None,
        }
    }

    // ── destinations ────────────────────────────────────────────────────────────────────────

    fn dest_page(&self, dest: &Object, depth: u32) -> Option<usize> {
        if depth > 8 {
            return None;
        }
        match self.resolve(dest) {
            Object::Array(a) => a.first().and_then(|p| self.page_of(p)),
            Object::Dictionary(d) => d.get(b"D").ok().and_then(|d| self.dest_page(d, depth + 1)),
            named @ (Object::String(..) | Object::Name(_)) => {
                let key = match named {
                    Object::String(s, _) => s.clone(),
                    Object::Name(n) => n.clone(),
                    _ => unreachable!(),
                };
                self.named_dest(&key).and_then(|d| self.dest_page(d, depth + 1))
            }
            _ => None,
        }
    }

    fn named_dest(&self, key: &[u8]) -> Option<&'a Object> {
        let catalog = self.doc.catalog().ok()?;
        if let Some(tree) = catalog.get(b"Names").ok().and_then(|o| self.dict(o)).and_then(|n| n.get(b"Dests").ok()).and_then(|o| self.dict(o)) {
            let mut seen = HashSet::new();
            if let Some((_, v)) = self.name_tree(tree, &mut seen, 0).into_iter().find(|(k, _)| k.as_bytes() == key) {
                return Some(v);
            }
        }
        // PDF 1.1 style /Dests dictionary.
        catalog.get(b"Dests").ok().and_then(|o| self.dict(o)).and_then(|d| d.get(key).ok())
    }

    fn name_tree(&self, node: &'a Dictionary, seen: &mut HashSet<*const Dictionary>, depth: u32) -> Vec<(String, &'a Object)> {
        let mut out = Vec::new();
        if depth > 32 || !seen.insert(node as *const _) {
            return out;
        }
        if let Ok(Object::Array(pairs)) = node.get(b"Names").map(|o| self.resolve(o)) {
            for pair in pairs.chunks(2) {
                if let [k, v] = pair {
                    let key = lopdf::decode_text_string(self.resolve(k)).unwrap_or_default();
                    out.push((key, v));
                }
            }
        }
        if let Ok(Object::Array(kids)) = node.get(b"Kids").map(|o| self.resolve(o)) {
            for k in kids {
                if let Some(d) = self.dict(k) {
                    out.extend(self.name_tree(d, seen, depth + 1));
                }
            }
        }
        out
    }

    // ── outline ─────────────────────────────────────────────────────────────────────────────

    fn outline_siblings(&self, first: &'a Object, seen: &mut HashSet<ObjectId>, depth: u32) -> Vec<OutlineItem> {
        let mut items = Vec::new();
        let mut cur = Some(first);
        while let Some(o) = cur {
            if let Object::Reference(id) = o
                && !seen.insert(*id)
            {
                break; // cycle
            }
            let Some(d) = self.dict(o) else { break };
            let page = d
                .get(b"Dest")
                .ok()
                .and_then(|dest| self.dest_page(dest, 0))
                .or_else(|| d.get(b"A").ok().and_then(|a| self.dict(a)).and_then(|a| a.get(b"D").ok()).and_then(|dest| self.dest_page(dest, 0)));
            let children = match (d.get(b"First").ok(), depth < 32) {
                (Some(f), true) => self.outline_siblings(f, seen, depth + 1),
                _ => Vec::new(),
            };
            let open = d.get(b"Count").ok().and_then(|c| c.as_i64().ok()).is_some_and(|c| c > 0);
            items.push(OutlineItem { title: self.text(d, b"Title").unwrap_or_default(), page, children, open });
            cur = d.get(b"Next").ok();
            if items.len() > 100_000 {
                break;
            }
        }
        items
    }

    // ── page labels (ISO 32000-2 §12.4.2) ───────────────────────────────────────────────────

    fn page_labels(&self, catalog: &Dictionary, pages: &mut [PageInfo]) {
        let Some(tree) = catalog.get(b"PageLabels").ok().and_then(|o| self.dict(o)) else { return };
        let mut ranges: BTreeMap<usize, &Dictionary> = BTreeMap::new();
        let mut stack = vec![(tree, 0u32)];
        while let Some((node, depth)) = stack.pop() {
            if let Ok(Object::Array(nums)) = node.get(b"Nums").map(|o| self.resolve(o)) {
                for pair in nums.chunks(2) {
                    if let [k, v] = pair
                        && let (Ok(start), Some(d)) = (self.resolve(k).as_i64(), self.dict(v))
                        && start >= 0
                    {
                        ranges.insert(start as usize, d);
                    }
                }
            }
            if depth < 32
                && let Ok(Object::Array(kids)) = node.get(b"Kids").map(|o| self.resolve(o))
            {
                stack.extend(kids.iter().filter_map(|k| self.dict(k)).map(|d| (d, depth + 1)));
            }
        }
        for (i, page) in pages.iter_mut().enumerate() {
            let Some((&start, d)) = ranges.range(..=i).next_back() else { continue };
            let first = d.get(b"St").ok().and_then(|o| o.as_i64().ok()).unwrap_or(1).max(1) as usize;
            let n = first + (i - start);
            let prefix = self.text(d, b"P").unwrap_or_default();
            let number = match self.name(d, b"S").as_deref() {
                Some("D") => n.to_string(),
                Some("R") => roman(n).to_uppercase(),
                Some("r") => roman(n),
                Some("A") => alpha(n).to_uppercase(),
                Some("a") => alpha(n),
                _ => String::new(),
            };
            page.label = format!("{prefix}{number}");
            if page.label.is_empty() {
                page.label = (i + 1).to_string();
            }
        }
    }

    // ── annotations ─────────────────────────────────────────────────────────────────────────

    fn annotations(&self, info: &mut DocInfo) {
        for (&id, &page) in &self.page_index {
            let Ok(page_dict) = self.doc.get_dictionary(id) else { continue };
            let Ok(Object::Array(annots)) = page_dict.get(b"Annots").map(|o| self.resolve(o)) else { continue };
            for a in annots {
                let Some(d) = self.dict(a) else { continue };
                let Some(subtype) = self.name(d, b"Subtype") else { continue };
                if subtype == "Link" {
                    if let Some(link) = self.link(page, d) {
                        if matches!(&link.target, LinkTarget::Other(s) if s == "JavaScript") {
                            info.has_javascript = true;
                        }
                        info.links.push(link);
                    }
                    continue;
                }
                if matches!(subtype.as_str(), "Widget" | "Popup") {
                    continue;
                }
                let rect = rect4(self.resolve(d.get(b"Rect").unwrap_or(&Object::Null)));
                let color = match d.get(b"C").map(|o| self.resolve(o)) {
                    Ok(Object::Array(c)) if c.len() == 3 => {
                        let f = |i: usize| c[i].as_float().unwrap_or(0.0);
                        Some([f(0), f(1), f(2)])
                    }
                    _ => None,
                };
                let in_reply_to = d.get(b"IRT").ok().and_then(|o| self.dict(o)).and_then(|p| self.text(p, b"NM"));
                info.annotations.push(Annotation {
                    page,
                    subtype,
                    author: self.text(d, b"T"),
                    contents: self.text(d, b"Contents"),
                    modified: self.text(d, b"M").map(|m| pretty_date(&m)),
                    name: self.text(d, b"NM"),
                    in_reply_to,
                    rect,
                    color,
                });
            }
        }
        info.annotations.sort_by(|a, b| a.page.cmp(&b.page).then(b.rect[3].total_cmp(&a.rect[3])));
    }

    fn link(&self, page: usize, d: &Dictionary) -> Option<Link> {
        let rect = rect4(self.resolve(d.get(b"Rect").ok()?));
        let target = if let Ok(dest) = d.get(b"Dest") {
            LinkTarget::Page(self.dest_page(dest, 0)?)
        } else {
            let a = d.get(b"A").ok().and_then(|a| self.dict(a))?;
            match self.name(a, b"S").as_deref() {
                Some("GoTo") => LinkTarget::Page(self.dest_page(a.get(b"D").ok()?, 0)?),
                Some("URI") => {
                    LinkTarget::Uri(a.get(b"URI").ok().and_then(|u| self.resolve(u).as_str().ok()).map(|b| String::from_utf8_lossy(b).into_owned())?)
                }
                Some(other) => LinkTarget::Other(other.to_string()),
                None => return None,
            }
        };
        Some(Link { page, rect, target })
    }

    // ── form fields ─────────────────────────────────────────────────────────────────────────

    fn field(
        &self,
        o: &'a Object,
        parent: Option<(&str, Option<String>, Option<i64>)>,
        out: &mut Vec<Field>,
        seen: &mut HashSet<ObjectId>,
        depth: u32,
    ) {
        if depth > 32 {
            return;
        }
        if let Object::Reference(id) = o
            && !seen.insert(*id)
        {
            return;
        }
        let Some(d) = self.dict(o) else { return };
        let (parent_name, parent_ft, parent_ff) = parent.unwrap_or(("", None, None));
        let partial = self.text(d, b"T");
        let full = match (&partial, parent_name.is_empty()) {
            (Some(t), true) => t.clone(),
            (Some(t), false) => format!("{parent_name}.{t}"),
            (None, _) => parent_name.to_string(),
        };
        let ft = self.name(d, b"FT").or(parent_ft);
        let ff = d.get(b"Ff").ok().and_then(|x| x.as_i64().ok()).or(parent_ff);
        // Non-terminal: has kids that are fields (they carry /T). Widgets-only kids are terminal.
        let kids = match d.get(b"Kids").map(|k| self.resolve(k)) {
            Ok(Object::Array(k)) => k.as_slice(),
            _ => &[],
        };
        let field_kids: Vec<_> = kids.iter().filter(|k| self.dict(k).is_some_and(|kd| kd.get(b"T").is_ok())).collect();
        if !field_kids.is_empty() {
            for k in field_kids {
                self.field(k, Some((&full, ft.clone(), ff)), out, seen, depth + 1);
            }
            return;
        }
        let flags = ff.unwrap_or(0);
        let kind = match ft.as_deref() {
            Some("Tx") => FieldKind::Text,
            Some("Btn") if flags & (1 << 16) != 0 => FieldKind::PushButton,
            Some("Btn") if flags & (1 << 15) != 0 => FieldKind::Radio,
            Some("Btn") => FieldKind::CheckBox,
            Some("Ch") if flags & (1 << 17) != 0 => FieldKind::Combo,
            Some("Ch") => FieldKind::List,
            Some("Sig") => FieldKind::Signature,
            _ => FieldKind::Unknown,
        };
        let value = match d.get(b"V").map(|v| self.resolve(v)) {
            Ok(Object::Name(n)) => Some(String::from_utf8_lossy(n).into_owned()),
            Ok(v @ Object::String(..)) => lopdf::decode_text_string(v).ok(),
            Ok(Object::Array(a)) => Some(a.iter().filter_map(|x| lopdf::decode_text_string(self.resolve(x)).ok()).collect::<Vec<_>>().join(", ")),
            Ok(Object::Dictionary(_)) if kind == FieldKind::Signature => Some("signed".into()),
            _ => None,
        };
        // The widget is either this dict (merged field/widget) or its first kid.
        let widget = if d.get(b"Rect").is_ok() { Some(d) } else { kids.first().and_then(|k| self.dict(k)) };
        let page = widget.and_then(|w| w.get(b"P").ok()).and_then(|p| self.page_of(p)).or_else(|| self.find_widget_page(o, kids));
        let has_actions = d.get(b"AA").is_ok() || widget.is_some_and(|w| w.get(b"AA").is_ok() || w.get(b"A").is_ok());
        let rect = widget.and_then(|w| w.get(b"Rect").ok()).map(|r| rect4(self.resolve(r)));
        out.push(Field { name: full, kind, value, page, tooltip: self.text(d, b"TU"), has_actions, rect });
    }

    fn find_widget_page(&self, field: &Object, kids: &[Object]) -> Option<usize> {
        let targets: HashSet<ObjectId> =
            std::iter::once(field).chain(kids.iter()).filter_map(|o| if let Object::Reference(id) = o { Some(*id) } else { None }).collect();
        if targets.is_empty() {
            return None;
        }
        self.page_index.iter().find_map(|(&pid, &idx)| {
            let annots = self.doc.get_dictionary(pid).ok()?.get(b"Annots").ok()?;
            let arr = self.resolve(annots).as_array().ok()?;
            arr.iter().any(|a| matches!(a, Object::Reference(id) if targets.contains(id))).then_some(idx)
        })
    }

    // ── optional content ────────────────────────────────────────────────────────────────────

    fn layers(&self, catalog: &Dictionary, out: &mut Vec<Layer>) {
        let Some(props) = catalog.get(b"OCProperties").ok().and_then(|o| self.dict(o)) else { return };
        let config = props.get(b"D").ok().and_then(|o| self.dict(o));
        let off: HashSet<ObjectId> = config
            .and_then(|c| c.get(b"OFF").ok())
            .and_then(|o| self.resolve(o).as_array().ok())
            .map(|a| a.iter().filter_map(|x| x.as_reference().ok()).collect())
            .unwrap_or_default();
        let base_off = config.and_then(|c| self.name(c, b"BaseState")).as_deref() == Some("OFF");
        let on: HashSet<ObjectId> = config
            .and_then(|c| c.get(b"ON").ok())
            .and_then(|o| self.resolve(o).as_array().ok())
            .map(|a| a.iter().filter_map(|x| x.as_reference().ok()).collect())
            .unwrap_or_default();
        let Ok(Object::Array(ocgs)) = props.get(b"OCGs").map(|o| self.resolve(o)) else { return };
        for g in ocgs {
            let Some(d) = self.dict(g) else { continue };
            let id = g.as_reference().ok();
            let visible = match id {
                Some(id) if off.contains(&id) => false,
                Some(id) if on.contains(&id) => true,
                _ => !base_off,
            };
            out.push(Layer { name: self.text(d, b"Name").unwrap_or_else(|| "Layer".into()), visible });
        }
    }

    fn attachment(&self, name: String, spec: &Object) -> Attachment {
        let d = self.dict(spec);
        let size =
            d.and_then(|d| d.get(b"EF").ok()).and_then(|o| self.dict(o)).and_then(|ef| ef.get(b"F").ok().or_else(|| ef.get(b"UF").ok())).and_then(
                |f| match self.resolve(f) {
                    Object::Stream(s) => s
                        .dict
                        .get(b"Params")
                        .ok()
                        .and_then(|p| self.dict(p))
                        .and_then(|p| p.get(b"Size").ok())
                        .and_then(|s| s.as_i64().ok())
                        .map(|s| s as usize)
                        .or(Some(s.content.len())),
                    _ => None,
                },
            );
        let display = d.and_then(|d| self.text(d, b"UF").or_else(|| self.text(d, b"F"))).unwrap_or(name);
        Attachment { name: display, description: d.and_then(|d| self.text(d, b"Desc")), size }
    }
}

fn rect4(o: &Object) -> [f32; 4] {
    match o {
        Object::Array(a) if a.len() == 4 => {
            let v: Vec<f32> = a.iter().map(|x| x.as_float().unwrap_or(0.0)).collect();
            [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])]
        }
        _ => [0.0; 4],
    }
}

fn roman(mut n: usize) -> String {
    const T: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// a..z, aa..zz, aaa.. (ISO 32000-2: "a to z for the first 26 pages, aa to zz for the next 26").
fn alpha(n: usize) -> String {
    let n = n.max(1) - 1;
    let letter = (b'a' + (n % 26) as u8) as char;
    std::iter::repeat_n(letter, n / 26 + 1).collect()
}

/// `D:20260930104512-04'00'` → `2026-09-30 10:45`.
fn pretty_date(s: &str) -> String {
    let d = s.trim_start_matches("D:");
    if d.len() >= 12 && d[..12].bytes().all(|b| b.is_ascii_digit()) {
        format!("{}-{}-{} {}:{}", &d[0..4], &d[4..6], &d[6..8], &d[8..10], &d[10..12])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roman_and_alpha_labels() {
        assert_eq!(roman(4), "iv");
        assert_eq!(roman(1994), "mcmxciv");
        assert_eq!(alpha(1), "a");
        assert_eq!(alpha(27), "aa");
        assert_eq!(alpha(53), "aaa");
    }

    #[test]
    fn dates_are_prettified() {
        assert_eq!(pretty_date("D:20260930104512-04'00'"), "2026-09-30 10:45");
        assert_eq!(pretty_date("yesterday"), "yesterday");
    }
}
