//! printcraft-render — page rasterization and document inspection.
//!
//! **Bootstrap status (see ADR-0004):** rasterization goes straight through the `hayro` crate
//! (hayro-interpret + vello_cpu), and inspection (outline, annotations, fields, layers,
//! attachments, metadata) uses `lopdf`. Both are replaced by our own `cos`/`model` crates and the
//! DisplayList device design in M1–M2. The public API here is what the engine and UI rely on, so
//! the swap stays internal to this crate.

mod inspect;
mod raster;

pub use inspect::{Annotation, Attachment, DocInfo, Field, FieldKind, Layer, Link, LinkTarget, OutlineItem, PageInfo, inspect};
pub use raster::{RenderPool, RenderRequest, RenderedPage};

/// Errors surfaced to the user when a document cannot be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("the file is not a readable PDF: {0}")]
    Invalid(String),
    #[error("the document is encrypted and needs a password")]
    NeedsPassword,
}
