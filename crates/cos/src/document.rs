//! Document: cross-reference data, lazy object loading, and a copy-on-write overlay of edits.
//!
//! `Document` is cheap to clone (the original bytes, xref index and parse caches are shared), so
//! a clone is a snapshot: undo/redo and background jobs hold clones while the UI edits another.
//! Edits never touch the original bytes; the writer appends them (incremental save) or rewrites
//! the file (full save).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::CosError;
use crate::object::{Dict, ObjRef, Object};
use crate::parser::{Lexer, is_whitespace, parse_indirect};

thread_local! {
    static LOAD_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Nesting counter for object loads on this thread (see `Document::try_get`).
struct LoadGuard;

impl LoadGuard {
    const MAX: u32 = 32;

    fn enter() -> Option<Self> {
        LOAD_DEPTH.with(|d| {
            if d.get() >= Self::MAX {
                return None;
            }
            d.set(d.get() + 1);
            Some(LoadGuard)
        })
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        LOAD_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefEntry {
    Free { next_generation: u16 },
    InFile { offset: u64, generation: u16 },
    InStream { stream: u32, index: u32 },
}

/// One cross-reference section of the file (one per save; the first is the original).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Offset of the xref section (`startxref` value).
    pub xref_offset: u64,
    /// `true` if the section is a cross-reference stream.
    pub is_stream: bool,
}

#[derive(Clone, Debug)]
struct ObjStm {
    /// (object number, byte offset within `data`).
    index: Vec<(u32, usize)>,
    data: Arc<Vec<u8>>,
}

/// Where an overlay slot came from.
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Set(u16, Arc<Object>),
    Freed(u16),
}

#[derive(Clone)]
pub struct Document {
    data: Arc<Vec<u8>>,
    entries: Arc<BTreeMap<u32, XrefEntry>>,
    trailer: Dict,
    revisions: Arc<Vec<Revision>>,
    repair_log: Arc<Vec<String>>,
    cache: Arc<Mutex<HashMap<u32, Arc<Object>>>>,
    objstms: Arc<Mutex<HashMap<u32, Arc<ObjStm>>>>,
    overlay: BTreeMap<u32, Slot>,
    next_num: u32,
    /// Position of the `%PDF-` header (some files have junk before it).
    header_offset: usize,
    version: String,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document").field("bytes", &self.data.len()).field("objects", &self.entries.len()).field("edits", &self.overlay.len()).finish()
    }
}

impl Document {
    /// Parse a document. Damaged cross-reference data is reconstructed; see `repair_log`.
    pub fn open(data: Arc<Vec<u8>>) -> Result<Self, CosError> {
        // Viewers accept files whose header is missing or damaged as long as the body looks
        // like PDF; so do we (a note goes to the repair log).
        let header = find(&data, b"%PDF-", 0, 1024);
        let headerless = header.is_none();
        if headerless && find(&data, b" obj", 0, 4096).is_none() {
            return Err(CosError::NotPdf);
        }
        let header_offset = header.unwrap_or(0);
        let version = match header {
            Some(h) => String::from_utf8_lossy(&data[(h + 5).min(data.len())..(h + 8).min(data.len())]).trim().to_string(),
            None => "1.4".to_string(),
        };
        let mut doc = Document {
            data,
            entries: Arc::new(BTreeMap::new()),
            trailer: Dict::new(),
            revisions: Arc::new(Vec::new()),
            repair_log: Arc::new(Vec::new()),
            cache: Arc::default(),
            objstms: Arc::default(),
            overlay: BTreeMap::new(),
            next_num: 1,
            header_offset,
            version,
        };
        let mut log = Vec::new();
        if headerless {
            log.push("the %PDF- header is missing; reading the file as PDF 1.4".into());
        }
        let parsed = doc.read_xref_chain(&mut log);
        let ok = match parsed {
            Ok((entries, trailer, revisions)) => {
                doc.entries = Arc::new(entries);
                doc.trailer = trailer;
                doc.revisions = Arc::new(revisions);
                doc.root_is_catalog()
            }
            Err(e) => {
                log.push(format!("cross-reference data unreadable ({e}); rebuilding from object headers"));
                false
            }
        };
        if !ok {
            if !doc.entries.is_empty() {
                log.push("cross-reference data does not lead to a valid catalog; rebuilding from object headers".into());
            }
            doc.reconstruct(&mut log)?;
        }
        doc.repair_log = Arc::new(log);
        if doc.trailer.contains(b"Encrypt") {
            return Err(CosError::Encrypted);
        }
        let max = doc.entries.keys().next_back().copied().unwrap_or(0);
        let size = doc.trailer.int(b"Size").unwrap_or(0).max(0) as u32;
        doc.next_num = max.max(size.saturating_sub(1)) + 1;
        Ok(doc)
    }

    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.data
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn trailer(&self) -> &Dict {
        &self.trailer
    }

    pub fn trailer_mut(&mut self) -> &mut Dict {
        &mut self.trailer
    }

    pub fn revisions(&self) -> &[Revision] {
        &self.revisions
    }

    pub fn repair_log(&self) -> &[String] {
        &self.repair_log
    }

    /// `true` when there are unsaved edits.
    pub fn is_modified(&self) -> bool {
        !self.overlay.is_empty()
    }

    /// Object numbers changed since the document was opened (or last saved).
    pub fn modified_objects(&self) -> Vec<u32> {
        self.overlay.keys().copied().collect()
    }

    pub fn root(&self) -> Option<ObjRef> {
        self.trailer.reference(b"Root")
    }

    // ── object access ───────────────────────────────────────────────────────────────────────

    /// Fetch an object by reference. Missing objects are `Null` (ISO 32000-2 §7.3.10).
    pub fn get(&self, r: ObjRef) -> Arc<Object> {
        self.try_get(r.num).unwrap_or_else(|_| Arc::new(Object::Null))
    }

    /// Follow a reference if `o` is one; otherwise return `o` itself.
    pub fn resolve(&self, o: &Object) -> Arc<Object> {
        match o {
            Object::Ref(r) => self.get(*r),
            other => Arc::new(other.clone()),
        }
    }

    /// Resolve and return a dictionary (a stream's dictionary counts).
    pub fn dict(&self, o: &Object) -> Option<Dict> {
        self.resolve(o).as_dict().cloned()
    }

    pub fn try_get(&self, num: u32) -> Result<Arc<Object>, CosError> {
        match self.overlay.get(&num) {
            Some(Slot::Set(_, o)) => return Ok(o.clone()),
            Some(Slot::Freed(_)) => return Ok(Arc::new(Object::Null)),
            None => {}
        }
        if let Some(o) = self.cache.lock().map_err(|_| CosError::Poisoned)?.get(&num) {
            return Ok(o.clone());
        }
        // Loading can re-enter (indirect stream `/Length`, object streams); damaged files make
        // cycles such as `6 0 obj << /Length 6 0 R >>`. Bound the nesting on every path.
        let _guard =
            LoadGuard::enter().ok_or_else(|| CosError::Syntax { offset: 0, detail: format!("reference cycle while loading object {num}") })?;
        let obj = self.load(num, 0)?;
        let obj = Arc::new(obj);
        self.cache.lock().map_err(|_| CosError::Poisoned)?.insert(num, obj.clone());
        Ok(obj)
    }

    fn load(&self, num: u32, depth: u32) -> Result<Object, CosError> {
        if depth > 16 {
            return Err(CosError::Syntax { offset: 0, detail: "reference cycle while loading".into() });
        }
        match self.entries.get(&num) {
            None | Some(XrefEntry::Free { .. }) => Ok(Object::Null),
            Some(XrefEntry::InFile { offset, .. }) => {
                let resolve = |r: ObjRef| self.try_get(r.num).ok().and_then(|o| o.as_int());
                let off = *offset as usize;
                match parse_indirect(&self.data, off, &resolve) {
                    Ok((id, o)) if id.num == num => Ok(o),
                    // Offsets relative to a shifted header, or simply wrong: try both fixes.
                    _ => match parse_indirect(&self.data, off + self.header_offset, &resolve) {
                        Ok((id, o)) if id.num == num => Ok(o),
                        _ => self.scan_for(num).ok_or(CosError::MissingObject(num)),
                    },
                }
            }
            Some(XrefEntry::InStream { stream, index }) => {
                let stm = self.objstm(*stream)?;
                let (n, off) = stm
                    .index
                    .get(*index as usize)
                    .copied()
                    .or_else(|| stm.index.iter().find(|(n, _)| *n == num).copied())
                    .ok_or(CosError::MissingObject(num))?;
                let off =
                    if n == num { off } else { stm.index.iter().find(|(n, _)| *n == num).map(|(_, o)| *o).ok_or(CosError::MissingObject(num))? };
                Lexer::new(&stm.data, off).object()
            }
        }
    }

    fn objstm(&self, num: u32) -> Result<Arc<ObjStm>, CosError> {
        if let Some(s) = self.objstms.lock().map_err(|_| CosError::Poisoned)?.get(&num) {
            return Ok(s.clone());
        }
        let Object::Stream(s) = &*self.try_get(num)? else { return Err(CosError::MissingObject(num)) };
        let n = s.dict.int(b"N").unwrap_or(0).clamp(0, 1_000_000) as usize;
        let first = s.dict.int(b"First").unwrap_or(0).max(0) as usize;
        let data = s.decoded()?;
        let mut lx = Lexer::new(&data, 0);
        let mut index = Vec::with_capacity(n);
        for _ in 0..n {
            let (Some(Object::Int(onum)), Some(Object::Int(off))) = (lx.object().ok(), lx.object().ok()) else { break };
            index.push((onum.max(0) as u32, first + off.max(0) as usize));
        }
        let stm = Arc::new(ObjStm { index, data: Arc::new(data) });
        self.objstms.lock().map_err(|_| CosError::Poisoned)?.insert(num, stm.clone());
        Ok(stm)
    }

    /// Last-resort lookup: scan the file for `num G obj`.
    fn scan_for(&self, num: u32) -> Option<Object> {
        let needle = format!("{num} ");
        let data = &self.data;
        let mut found = None;
        let mut i = 0;
        while let Some(p) = find(data, needle.as_bytes(), i, data.len()) {
            i = p + 1;
            if p > 0 && !is_whitespace(data[p - 1]) {
                continue;
            }
            if let Ok((id, o)) = parse_indirect(data, p, &|_| None)
                && id.num == num
            {
                found = Some(o); // keep the last (newest) definition
            }
        }
        found
    }

    // ── editing ─────────────────────────────────────────────────────────────────────────────

    /// Replace an object (keeping its generation).
    pub fn set(&mut self, r: ObjRef, obj: impl Into<Object>) {
        self.overlay.insert(r.num, Slot::Set(r.generation, Arc::new(obj.into())));
        if r.num >= self.next_num {
            self.next_num = r.num + 1;
        }
    }

    /// Add a new indirect object and return its reference.
    pub fn add(&mut self, obj: impl Into<Object>) -> ObjRef {
        let r = ObjRef::new(self.next_num, 0);
        self.next_num += 1;
        self.overlay.insert(r.num, Slot::Set(0, Arc::new(obj.into())));
        r
    }

    /// Delete an object (its number becomes free with the next generation).
    pub fn free(&mut self, r: ObjRef) {
        self.overlay.insert(r.num, Slot::Freed(r.generation.saturating_add(1)));
    }

    /// Edit a dictionary (or stream dictionary) object in place.
    pub fn update_dict(&mut self, r: ObjRef, f: impl FnOnce(&mut Dict)) -> Result<(), CosError> {
        let mut obj = (*self.get(r)).clone();
        let d = obj.as_dict_mut().ok_or(CosError::NotADictionary(r))?;
        f(d);
        self.set(r, obj);
        Ok(())
    }

    /// Current generation of an object number.
    pub fn generation(&self, num: u32) -> u16 {
        match self.overlay.get(&num) {
            Some(Slot::Set(g, _)) => *g,
            Some(Slot::Freed(g)) => *g,
            None => match self.entries.get(&num) {
                Some(XrefEntry::InFile { generation, .. }) => *generation,
                Some(XrefEntry::Free { next_generation }) => *next_generation,
                _ => 0,
            },
        }
    }

    pub(crate) fn overlay_entries(&self) -> impl Iterator<Item = (u32, u16, Option<Arc<Object>>)> + '_ {
        self.overlay.iter().map(|(n, s)| match s {
            Slot::Set(g, o) => (*n, *g, Some(o.clone())),
            Slot::Freed(g) => (*n, *g, None),
        })
    }

    pub(crate) fn next_num(&self) -> u32 {
        self.next_num
    }

    /// All object numbers known (file and overlay), excluding freed ones.
    pub fn object_numbers(&self) -> Vec<u32> {
        let mut set: HashSet<u32> = self.entries.iter().filter(|(_, e)| !matches!(e, XrefEntry::Free { .. })).map(|(n, _)| *n).collect();
        for (n, s) in &self.overlay {
            match s {
                Slot::Set(..) => set.insert(*n),
                Slot::Freed(_) => set.remove(n),
            };
        }
        let mut v: Vec<u32> = set.into_iter().collect();
        v.sort_unstable();
        v
    }

    /// Rebase this document on bytes written by the writer (after a save): edits become part of
    /// the file and the overlay is cleared, so the next incremental save appends only new edits.
    pub fn reopen_after_save(&self, bytes: Arc<Vec<u8>>) -> Result<Self, CosError> {
        Document::open(bytes)
    }

    // ── cross-reference parsing ─────────────────────────────────────────────────────────────

    fn root_is_catalog(&self) -> bool {
        let Some(r) = self.root() else { return false };
        matches!(self.try_get(r.num).as_deref(), Ok(Object::Dict(d)) if d.name(b"Type") == Some(b"Catalog") || d.contains(b"Pages"))
    }

    #[allow(clippy::type_complexity)]
    fn read_xref_chain(&self, log: &mut Vec<String>) -> Result<(BTreeMap<u32, XrefEntry>, Dict, Vec<Revision>), CosError> {
        let data = &self.data;
        let tail_start = data.len().saturating_sub(4096);
        let sx = rfind(data, b"startxref", tail_start).ok_or(CosError::Syntax { offset: data.len(), detail: "no startxref".into() })?;
        let mut lx = Lexer::new(data, sx + 9);
        let first = match lx.object() {
            Ok(Object::Int(n)) if n >= 0 => n as u64,
            _ => return Err(CosError::Syntax { offset: sx, detail: "bad startxref value".into() }),
        };
        let mut entries = BTreeMap::new();
        let mut trailer: Option<Dict> = None;
        let mut revisions = Vec::new();
        let mut visited = HashSet::new();
        let mut next = Some(first);
        while let Some(off) = next.take() {
            if !visited.insert(off) || visited.len() > 10_000 {
                log.push(format!("cross-reference chain loops at offset {off}; stopped"));
                break;
            }
            let (sect_trailer, is_stream) = match self.read_section(off as usize, &mut entries) {
                Ok(v) => v,
                Err(e) if self.header_offset > 0 => {
                    log.push(format!("xref at {off} unreadable ({e}); retrying relative to the header"));
                    self.read_section(off as usize + self.header_offset, &mut entries)?
                }
                Err(e) => {
                    if revisions.is_empty() {
                        return Err(e);
                    }
                    log.push(format!("older cross-reference section at {off} is unreadable: {e}"));
                    break;
                }
            };
            revisions.push(Revision { xref_offset: off, is_stream });
            // Hybrid files: the /XRefStm entries supplement the table (§7.5.8.4).
            if let Some(x) = sect_trailer.int(b"XRefStm").filter(|x| *x >= 0)
                && let Err(e) = self.read_section(x as usize, &mut entries)
            {
                log.push(format!("hybrid /XRefStm at {x} unreadable: {e}"));
            }
            next = sect_trailer.int(b"Prev").filter(|p| *p >= 0).map(|p| p as u64);
            if trailer.is_none() {
                trailer = Some(sect_trailer);
            }
        }
        revisions.reverse();
        let mut trailer = trailer.ok_or(CosError::Syntax { offset: 0, detail: "no trailer".into() })?;
        for k in [&b"Prev"[..], b"XRefStm", b"Type", b"W", b"Index", b"Filter", b"DecodeParms", b"Length"] {
            trailer.remove(k);
        }
        Ok((entries, trailer, revisions))
    }

    /// Read one section at `off`, adding entries not already known (newer sections win).
    fn read_section(&self, off: usize, entries: &mut BTreeMap<u32, XrefEntry>) -> Result<(Dict, bool), CosError> {
        let data = &self.data;
        let mut lx = Lexer::new(data, off);
        if lx.eat_keyword(b"xref") {
            loop {
                lx.skip_ws();
                if lx.eat_keyword(b"trailer") {
                    let t = lx.object()?;
                    let d = t.as_dict().cloned().ok_or(CosError::Syntax { offset: lx.pos, detail: "trailer is not a dictionary".into() })?;
                    return Ok((d, false));
                }
                let (Ok(Object::Int(start)), Ok(Object::Int(count))) = (lx.object(), lx.object()) else {
                    return Err(CosError::Syntax { offset: lx.pos, detail: "bad xref subsection header".into() });
                };
                if start < 0 || !(0..=10_000_000).contains(&count) {
                    return Err(CosError::Syntax { offset: lx.pos, detail: "implausible xref subsection".into() });
                }
                for i in 0..count {
                    lx.skip_ws();
                    let o = lx.token();
                    lx.skip_ws();
                    let g = lx.token();
                    lx.skip_ws();
                    let kind = lx.token();
                    let num = (start + i) as u32;
                    let offset = std::str::from_utf8(o).ok().and_then(|s| s.parse::<u64>().ok());
                    let generation = std::str::from_utf8(g).ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0).min(u16::MAX as u32) as u16;
                    let entry = match (kind, offset) {
                        (b"n", Some(offset)) => XrefEntry::InFile { offset, generation },
                        (b"f", _) => XrefEntry::Free { next_generation: generation },
                        _ => return Err(CosError::Syntax { offset: lx.pos, detail: "bad xref entry".into() }),
                    };
                    // Offset 0 "in use" entries are a known producer bug: ignore them.
                    if matches!(entry, XrefEntry::InFile { offset: 0, .. }) {
                        continue;
                    }
                    entries.entry(num).or_insert(entry);
                }
            }
        }
        // Cross-reference stream.
        let (_, obj) = parse_indirect(data, off, &|_| None)?;
        let Object::Stream(s) = obj else { return Err(CosError::Syntax { offset: off, detail: "expected xref table or stream".into() }) };
        if s.dict.name(b"Type") != Some(b"XRef") {
            return Err(CosError::Syntax { offset: off, detail: "stream at startxref is not /Type /XRef".into() });
        }
        let w: Vec<usize> = s
            .dict
            .get(b"W")
            .and_then(Object::as_array)
            .map(|a| a.iter().map(|x| x.as_int().unwrap_or(0).clamp(0, 8) as usize).collect())
            .unwrap_or_default();
        if w.len() < 3 {
            return Err(CosError::Syntax { offset: off, detail: "xref stream /W invalid".into() });
        }
        let size = s.dict.int(b"Size").unwrap_or(0).max(0);
        let index: Vec<i64> = match s.dict.get(b"Index").and_then(Object::as_array) {
            Some(a) => a.iter().filter_map(Object::as_int).collect(),
            None => vec![0, size],
        };
        let raw = s.decoded()?;
        let row = w[0] + w[1] + w[2];
        if row == 0 {
            return Err(CosError::Syntax { offset: off, detail: "xref stream row width 0".into() });
        }
        let field = |r: &[u8], from: usize, len: usize, default: u64| -> u64 {
            if len == 0 {
                return default;
            }
            r[from..from + len].iter().fold(0u64, |acc, b| acc << 8 | *b as u64)
        };
        let mut rows = raw.chunks_exact(row);
        for pair in index.chunks(2) {
            let [start, count] = pair else { break };
            for i in 0..(*count).max(0) {
                let Some(r) = rows.next() else { break };
                let t = field(r, 0, w[0], 1);
                let a = field(r, w[0], w[1], 0);
                let b = field(r, w[0] + w[1], w[2], 0);
                let num = (*start + i).max(0) as u32;
                let entry = match t {
                    0 => XrefEntry::Free { next_generation: b.min(u16::MAX as u64) as u16 },
                    1 => XrefEntry::InFile { offset: a, generation: b.min(u16::MAX as u64) as u16 },
                    2 => XrefEntry::InStream { stream: a.min(u32::MAX as u64) as u32, index: b.min(u32::MAX as u64) as u32 },
                    _ => continue, // reserved types are treated as null references (§7.5.8.3)
                };
                if matches!(entry, XrefEntry::InFile { offset: 0, .. }) {
                    continue;
                }
                entries.entry(num).or_insert(entry);
            }
        }
        Ok((s.dict.clone(), true))
    }

    /// Rebuild the cross-reference index by scanning for `N G obj` headers (§7.5.4 recovery).
    fn reconstruct(&mut self, log: &mut Vec<String>) -> Result<(), CosError> {
        let data = self.data.clone();
        let mut entries: BTreeMap<u32, XrefEntry> = BTreeMap::new();
        let mut i = 0;
        let mut objstms = Vec::new();
        while i < data.len() {
            // Object headers start at a line start (or after whitespace).
            let at_start = i == 0 || is_whitespace(data[i - 1]) || data[i - 1] == b'>';
            if at_start && data[i].is_ascii_digit() {
                let mut lx = Lexer::new(&data, i);
                let n = lx.token();
                lx.skip_ws();
                let g = lx.token();
                if !n.is_empty() && !g.is_empty() && n.iter().all(u8::is_ascii_digit) && g.iter().all(u8::is_ascii_digit) && lx.eat_keyword(b"obj") {
                    let num = std::str::from_utf8(n).ok().and_then(|s| s.parse::<u32>().ok());
                    let generation = std::str::from_utf8(g).ok().and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
                    if let Some(num) = num {
                        entries.insert(num, XrefEntry::InFile { offset: i as u64, generation });
                        objstms.push(num);
                    }
                    i = lx.pos;
                    continue;
                }
            }
            i += 1;
        }
        if entries.is_empty() {
            return Err(CosError::Syntax { offset: 0, detail: "no objects found".into() });
        }
        self.entries = Arc::new(entries.clone());
        self.cache.lock().map_err(|_| CosError::Poisoned)?.clear();
        // Objects inside object streams.
        for num in objstms {
            if let Ok(o) = self.try_get(num)
                && let Object::Stream(s) = &*o
                && s.dict.name(b"Type") == Some(b"ObjStm")
                && let Ok(stm) = self.objstm(num)
            {
                for (idx, (inner, _)) in stm.index.iter().enumerate() {
                    entries.entry(*inner).or_insert(XrefEntry::InStream { stream: num, index: idx as u32 });
                }
            }
        }
        self.entries = Arc::new(entries);
        self.cache.lock().map_err(|_| CosError::Poisoned)?.clear();
        // Trailer: the last `trailer` dictionary with a usable /Root, else the catalog itself.
        let mut trailer = None;
        let mut from = 0;
        while let Some(p) = find(&data, b"trailer", from, data.len()) {
            from = p + 7;
            let mut lx = Lexer::new(&data, from);
            if let Ok(Object::Dict(d)) = lx.object()
                && d.reference(b"Root").is_some_and(|r| matches!(self.try_get(r.num).as_deref(), Ok(Object::Dict(_))))
            {
                trailer = Some(d);
            }
        }
        let trailer = match trailer {
            Some(t) => t,
            None => {
                let catalog = self
                    .entries
                    .keys()
                    .copied()
                    .find(|n| matches!(self.try_get(*n).as_deref(), Ok(Object::Dict(d)) if d.name(b"Type") == Some(b"Catalog")));
                let Some(c) = catalog else { return Err(CosError::Syntax { offset: 0, detail: "no document catalog found".into() }) };
                let mut t = Dict::new();
                t.set(b"Root".to_vec(), Object::Ref(ObjRef::new(c, self.generation(c))));
                t
            }
        };
        let mut trailer = trailer;
        for k in [&b"Prev"[..], b"XRefStm"] {
            trailer.remove(k);
        }
        self.trailer = trailer;
        self.revisions = Arc::new(Vec::new());
        log.push(format!("rebuilt cross-reference index from {} object headers", self.entries.len()));
        Ok(())
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize, to: usize) -> Option<usize> {
    let to = to.min(hay.len());
    if from >= to || needle.is_empty() || to - from < needle.len() {
        return None;
    }
    hay[from..to].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn rfind(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a classic-xref file from object bodies (object i+1 = bodies[i]).
    pub(crate) fn build(bodies: &[&str], trailer: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, b) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{b}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
        out
    }

    #[test]
    fn self_referential_length_does_not_overflow() {
        let bytes = build(
            &["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Length 3 0 R >>\nstream\nabc\nendstream"],
            "/Root 1 0 R",
        );
        let doc = Document::open(Arc::new(bytes)).unwrap();
        // Either recovered via the endstream search or reported — but never a stack overflow.
        let o = doc.get(ObjRef::new(3, 0));
        if let Object::Stream(s) = o.as_ref() {
            assert_eq!(s.raw.as_slice(), b"abc");
        }
    }

    #[test]
    fn mutually_referential_lengths_do_not_overflow() {
        let bytes = build(
            &[
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [] /Count 0 >>",
                "<< /Length 4 0 R >>\nstream\nabc\nendstream",
                "<< /Length 3 0 R >>\nstream\nxyz\nendstream",
            ],
            "/Root 1 0 R",
        );
        let doc = Document::open(Arc::new(bytes)).unwrap();
        let _ = doc.get(ObjRef::new(3, 0));
        let _ = doc.get(ObjRef::new(4, 0));
    }

    #[test]
    fn reads_classic_revision_and_edits_are_copy_on_write() {
        let bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
        let doc = Document::open(Arc::new(bytes)).unwrap();
        assert_eq!(doc.revisions().len(), 1);
        assert!(doc.repair_log().is_empty());
        let mut edited = doc.clone();
        edited.update_dict(ObjRef::new(1, 0), |d| d.set(b"Lang".to_vec(), Object::String(crate::PdfString::literal("en")))).unwrap();
        assert!(edited.is_modified() && !doc.is_modified(), "clone is an independent snapshot");
        assert!(doc.get(ObjRef::new(1, 0)).as_dict().unwrap().get(b"Lang").is_none());
    }

    #[test]
    fn damaged_xref_is_reconstructed() {
        let mut bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
        let n = bytes.len();
        bytes[n - 12..n - 6].copy_from_slice(b"999999"); // point startxref into nowhere
        let doc = Document::open(Arc::new(bytes)).unwrap();
        assert!(doc.revisions().is_empty());
        assert!(!doc.repair_log().is_empty());
        assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
    }
}
