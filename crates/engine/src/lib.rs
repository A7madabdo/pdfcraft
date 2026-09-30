//! printcraft-engine — the façade every frontend talks to (architecture §4).
//!
//! Holds open documents, their edit history and the tool catalogue. Frontends never touch the
//! parsing, rendering or editing crates directly.
//!
//! **Editing model.** Each document keeps a `printcraft_cos::Document` (the object graph with a
//! copy-on-write overlay of edits). An edit runs on a clone, and on success the previous state is
//! pushed onto the undo stack (clones share all unchanged data, so this is cheap). After every
//! edit the *working file* is produced by an incremental write — original bytes plus one
//! appended revision — and the view is refreshed from it, so what you see is exactly what Save
//! will write. Saving rebases onto the written bytes, so the next save appends only new edits.

pub mod catalog;

use std::sync::Arc;

use printcraft_cos::{SaveOptions, write_full, write_incremental};
use printcraft_render::{DocInfo, OpenError, RenderConfig, RenderPool, inspect};

/// Stable identifier of an open document within a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocId(pub u64);

/// Undo/redo depth. Snapshots share unchanged data, so this is memory-cheap.
const MAX_UNDO: usize = 100;

/// Editing state of a document (absent when the document cannot be edited yet, e.g. encrypted).
#[derive(Clone)]
struct Editor {
    cos: printcraft_cos::Document,
    undo: Vec<(String, printcraft_cos::Document)>,
    redo: Vec<(String, printcraft_cos::Document)>,
}

pub struct Document {
    pub id: DocId,
    pub name: String,
    pub path: Option<String>,
    /// The working file: what Save writes and what is displayed.
    pub bytes: Arc<Vec<u8>>,
    pub info: DocInfo,
    pub renderer: RenderPool,
    /// The password the document was opened with (needed to read attachments, etc.).
    pub password: Option<String>,
    /// `true` when there are edits that have not been saved.
    pub dirty: bool,
    /// Why the document cannot be edited (e.g. encryption), if so.
    pub read_only_reason: Option<String>,
    editor: Option<Editor>,
    config: RenderConfig,
}

impl Document {
    pub fn can_undo(&self) -> Option<&str> {
        self.editor.as_ref().and_then(|e| e.undo.last()).map(|(l, _)| l.as_str())
    }

    pub fn can_redo(&self) -> Option<&str> {
        self.editor.as_ref().and_then(|e| e.redo.last()).map(|(l, _)| l.as_str())
    }

    pub fn editable(&self) -> bool {
        self.editor.is_some()
    }

    /// Current value of a document-information entry (Title, Author, …).
    pub fn info_value(&self, key: &str) -> Option<String> {
        self.editor.as_ref().and_then(|e| printcraft_organize::info(&e.cos, key))
    }

    /// Notes about damage repaired while opening (Document Properties ▸ Advanced, notices).
    pub fn repair_log(&self) -> Vec<String> {
        self.editor.as_ref().map(|e| e.cos.repair_log().to_vec()).unwrap_or_default()
    }
}

fn render_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8) - 1
}

/// Edits that can be applied to a document. Page indices are 0-based.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    RotatePages {
        pages: Vec<usize>,
        degrees: i64,
    },
    DeletePages {
        pages: Vec<usize>,
    },
    MovePages {
        pages: Vec<usize>,
        to: usize,
    },
    InsertBlankPage {
        at: usize,
        width: f64,
        height: f64,
    },
    SetInfo {
        key: String,
        value: String,
    },
    /// Several edits applied as one undoable step (all or nothing).
    Batch {
        label: String,
        edits: Vec<Edit>,
    },
}

impl Edit {
    /// Label for the Edit menu and history ("Undo Rotate pages").
    pub fn label(&self) -> String {
        match self {
            Edit::RotatePages { pages, .. } => plural("Rotate page", pages.len()),
            Edit::DeletePages { pages } => plural("Delete page", pages.len()),
            Edit::MovePages { pages, .. } => plural("Move page", pages.len()),
            Edit::InsertBlankPage { .. } => "Insert blank page".into(),
            Edit::SetInfo { key, .. } => format!("Change {key}"),
            Edit::Batch { label, .. } => label.clone(),
        }
    }
}

/// Perform an edit on a working copy (the caller discards it on error).
fn run_edit(doc: &mut printcraft_cos::Document, edit: &Edit) -> Result<(), EditError> {
    match edit {
        Edit::RotatePages { pages, degrees } => printcraft_organize::rotate_pages(doc, pages, *degrees)?,
        Edit::DeletePages { pages } => printcraft_organize::delete_pages(doc, pages)?,
        Edit::MovePages { pages, to } => printcraft_organize::move_pages(doc, pages, *to)?,
        Edit::InsertBlankPage { at, width, height } => {
            printcraft_organize::insert_blank_page(doc, *at, *width, *height)?;
        }
        Edit::SetInfo { key, value } => printcraft_organize::set_info(doc, key, value)?,
        Edit::Batch { edits, .. } => {
            for e in edits {
                run_edit(doc, e)?;
            }
        }
    }
    Ok(())
}

fn plural(s: &str, n: usize) -> String {
    if n == 1 { s.to_string() } else { format!("{s}s") }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("no such document")]
    NoDocument,
    #[error("this document can't be edited: {0}")]
    ReadOnly(String),
    #[error("{0}")]
    Organize(#[from] printcraft_organize::OrganizeError),
    #[error("the edited document could not be written: {0}")]
    Write(String),
    #[error("the edited document could not be reopened: {0}")]
    Reopen(String),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
}

#[derive(Default)]
pub struct Session {
    docs: Vec<Document>,
    next_id: u64,
    /// Seconds since the Unix epoch, injected so saves are deterministic in tests.
    clock: Option<fn() -> i64>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a fixed clock (tests) instead of the system time.
    pub fn with_clock(mut self, clock: fn() -> i64) -> Self {
        self.clock = Some(clock);
        self
    }

    fn now(&self) -> Option<i64> {
        if let Some(c) = self.clock {
            return Some(c());
        }
        #[cfg(not(target_arch = "wasm32"))]
        return std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64);
        #[cfg(target_arch = "wasm32")]
        return None;
    }

    /// Open a document from bytes. Rendering starts lazily when pages are requested.
    ///
    /// `password` is tried as either the user or owner password when the file is encrypted.
    pub fn open(&mut self, name: impl Into<String>, path: Option<String>, bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<DocId, OpenError> {
        let info = inspect(bytes.clone(), password)?;
        let config = RenderConfig { password: password.map(Arc::from), ..Default::default() };
        let renderer = RenderPool::new(bytes.clone(), render_threads(), config.clone());
        let (editor, read_only_reason) = match std::panic::catch_unwind(|| printcraft_cos::Document::open(bytes.clone())) {
            Ok(Ok(cos)) => (Some(Editor { cos, undo: Vec::new(), redo: Vec::new() }), None),
            Ok(Err(e)) => (None, Some(e.to_string())),
            Err(_) => (None, Some("the document structure could not be read for editing".into())),
        };
        self.next_id += 1;
        let id = DocId(self.next_id);
        self.docs.push(Document {
            id,
            name: name.into(),
            path,
            bytes,
            info,
            renderer,
            password: password.map(str::to_owned),
            dirty: false,
            read_only_reason,
            editor,
            config,
        });
        Ok(id)
    }

    fn doc_mut(&mut self, id: DocId) -> Result<&mut Document, EditError> {
        self.docs.iter_mut().find(|d| d.id == id).ok_or(EditError::NoDocument)
    }

    /// Apply an edit. On success the previous state is undoable and the view data is refreshed.
    pub fn apply(&mut self, id: DocId, edit: Edit) -> Result<(), EditError> {
        let doc = self.doc_mut(id)?;
        let reason = doc.read_only_reason.clone().unwrap_or_default();
        let editor = doc.editor.as_mut().ok_or(EditError::ReadOnly(reason))?;
        let mut next = editor.cos.clone();
        run_edit(&mut next, &edit)?;
        let previous = std::mem::replace(&mut editor.cos, next);
        editor.undo.push((edit.label(), previous));
        if editor.undo.len() > MAX_UNDO {
            editor.undo.remove(0);
        }
        editor.redo.clear();
        if let Err(e) = Self::refresh(doc) {
            // Roll back: the edit produced something we cannot display.
            if let Some(ed) = doc.editor.as_mut()
                && let Some((_, prev)) = ed.undo.pop()
            {
                ed.cos = prev;
            }
            let _ = Self::refresh(doc);
            return Err(e);
        }
        doc.dirty = true;
        Ok(())
    }

    pub fn undo(&mut self, id: DocId) -> Result<String, EditError> {
        let doc = self.doc_mut(id)?;
        let editor = doc.editor.as_mut().ok_or(EditError::NothingToUndo)?;
        let (label, prev) = editor.undo.pop().ok_or(EditError::NothingToUndo)?;
        let current = std::mem::replace(&mut editor.cos, prev);
        editor.redo.push((label.clone(), current));
        Self::refresh(doc)?;
        doc.dirty = true;
        Ok(label)
    }

    pub fn redo(&mut self, id: DocId) -> Result<String, EditError> {
        let doc = self.doc_mut(id)?;
        let editor = doc.editor.as_mut().ok_or(EditError::NothingToRedo)?;
        let (label, next) = editor.redo.pop().ok_or(EditError::NothingToRedo)?;
        let current = std::mem::replace(&mut editor.cos, next);
        editor.undo.push((label.clone(), current));
        Self::refresh(doc)?;
        doc.dirty = true;
        Ok(label)
    }

    /// Rebuild working bytes, inspection and renderer from the current edit state.
    fn refresh(doc: &mut Document) -> Result<(), EditError> {
        let Some(editor) = doc.editor.as_ref() else { return Ok(()) };
        let bytes = if editor.cos.is_modified() {
            Arc::new(write_incremental(&editor.cos, &SaveOptions::default()).map_err(|e| EditError::Write(e.to_string()))?)
        } else {
            editor.cos.bytes().clone()
        };
        let info = inspect(bytes.clone(), doc.password.as_deref()).map_err(|e| EditError::Reopen(e.to_string()))?;
        // Keep the user's layer choices where the layers still exist.
        let mut info = info;
        for l in &mut info.layers {
            if let Some(old) = doc.info.layers.iter().find(|o| o.id == l.id) {
                l.visible = old.visible;
            }
        }
        doc.info = info;
        doc.bytes = bytes.clone();
        doc.renderer = RenderPool::new(bytes, render_threads(), doc.config.clone());
        Ok(())
    }

    /// The bytes to write for Save: an incremental update of the file as opened/last saved,
    /// with `/ModDate` stamped. Call `mark_saved` after writing them successfully.
    pub fn save_bytes(&self, id: DocId) -> Result<Arc<Vec<u8>>, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        let Some(editor) = doc.editor.as_ref() else { return Ok(doc.bytes.clone()) };
        if !editor.cos.is_modified() {
            return Ok(editor.cos.bytes().clone());
        }
        let opts = SaveOptions { mod_date: self.now().map(printcraft_cos::pdf_date), ..SaveOptions::default() };
        write_incremental(&editor.cos, &opts).map(Arc::new).map_err(|e| EditError::Write(e.to_string()))
    }

    /// A compact, garbage-collected rewrite (Save As ▸ "Optimized" / Reduce File Size groundwork).
    pub fn save_full_bytes(&self, id: DocId) -> Result<Arc<Vec<u8>>, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        let editor = doc.editor.as_ref().ok_or_else(|| EditError::ReadOnly(doc.read_only_reason.clone().unwrap_or_default()))?;
        let opts = SaveOptions { mod_date: self.now().map(printcraft_cos::pdf_date), ..SaveOptions::default() };
        write_full(&editor.cos, &opts).map(Arc::new).map_err(|e| EditError::Write(e.to_string()))
    }

    /// Record a successful save of `bytes` (to `path`, if any): rebase editing on the saved file
    /// so the next save appends only newer edits. Undo history is kept.
    pub fn mark_saved(&mut self, id: DocId, bytes: Arc<Vec<u8>>, path: Option<String>) -> Result<(), EditError> {
        let doc = self.doc_mut(id)?;
        if let Some(editor) = doc.editor.as_mut() {
            editor.cos = printcraft_cos::Document::open(bytes.clone()).map_err(|e| EditError::Reopen(e.to_string()))?;
        }
        if let Some(p) = path {
            doc.name = std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
            doc.path = Some(p);
        }
        doc.dirty = false;
        Self::refresh(doc)
    }

    /// Show or hide a layer (optional content group) for viewing. Returns `true` if it changed;
    /// callers must drop cached rasters and text for the document.
    pub fn set_layer_visible(&mut self, id: DocId, layer: usize, visible: bool) -> bool {
        let Some(doc) = self.docs.iter_mut().find(|d| d.id == id) else { return false };
        let Some(l) = doc.info.layers.get_mut(layer) else { return false };
        if l.visible == visible {
            return false;
        }
        l.visible = visible;
        let overrides: Vec<(i32, i32, bool)> = doc.info.layers.iter().map(|l| (l.id.0 as i32, l.id.1 as i32, l.visible)).collect();
        doc.config.layers = Arc::new(overrides);
        doc.renderer = RenderPool::new(doc.bytes.clone(), render_threads(), doc.config.clone());
        true
    }

    pub fn close(&mut self, id: DocId) {
        self.docs.retain(|d| d.id != id);
    }

    pub fn get(&self, id: DocId) -> Option<&Document> {
        self.docs.iter().find(|d| d.id == id)
    }

    pub fn docs(&self) -> &[Document] {
        &self.docs
    }
}

#[cfg(test)]
mod tests;
