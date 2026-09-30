//! printcraft-engine — the façade every frontend talks to (architecture §4).
//!
//! This first slice holds open documents and the tool catalogue. The command registry, history
//! and jobs arrive in M4; the types here are shaped so that the UI never touches rendering or
//! parsing crates directly.

pub mod catalog;

use std::sync::Arc;

use printcraft_render::{DocInfo, OpenError, RenderPool, inspect};

/// Stable identifier of an open document within a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocId(pub u64);

pub struct Document {
    pub id: DocId,
    pub name: String,
    pub path: Option<String>,
    pub bytes: Arc<Vec<u8>>,
    pub info: DocInfo,
    pub renderer: RenderPool,
}

#[derive(Default)]
pub struct Session {
    docs: Vec<Document>,
    next_id: u64,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a document from bytes. Rendering starts lazily when pages are requested.
    pub fn open(&mut self, name: impl Into<String>, path: Option<String>, bytes: Vec<u8>) -> Result<DocId, OpenError> {
        let bytes = Arc::new(bytes);
        let info = inspect(bytes.clone())?;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8) - 1;
        let renderer = RenderPool::new(bytes.clone(), threads);
        self.next_id += 1;
        let id = DocId(self.next_id);
        self.docs.push(Document { id, name: name.into(), path, bytes, info, renderer });
        Ok(id)
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
