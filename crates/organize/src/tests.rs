use std::sync::Arc;

use printcraft_cos::{Document, Object, SaveOptions, write_full, write_incremental};

use super::*;

/// A 3-page document with a nested page tree. MediaBox and Rotate are inherited from the root,
/// Resources from an intermediate node; each page's content says which page it is.
fn fixture() -> Vec<u8> {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),                                                  // 1
        b"<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 3 /MediaBox [0 0 300 400] /Rotate 90 >>".to_vec(), // 2
        b"<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 /Resources << /Font << /F1 10 0 R >> >> >>".to_vec(), // 3
        b"<< /Type /Page /Parent 3 0 R /Contents 7 0 R >>".to_vec(),                                    // 4
        b"<< /Type /Page /Parent 3 0 R /Contents 8 0 R >>".to_vec(),                                    // 5
        b"<< /Type /Page /Parent 2 0 R /Contents 9 0 R /Resources << /Font << /F1 10 0 R >> >> /MediaBox [0 0 500 500] >>".to_vec(), // 6
        stream(b"BT /F1 12 Tf 20 20 Td (Page 1) Tj ET"),                                                // 7
        stream(b"BT /F1 12 Tf 20 20 Td (Page 2) Tj ET"),                                                // 8
        stream(b"BT /F1 12 Tf 20 20 Td (Page 3) Tj ET"),                                                // 9
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),                             // 10
        b"<< /Title (Original) /Producer (fixture) >>".to_vec(),                                        // 11
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 11 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn stream(body: &[u8]) -> Vec<u8> {
    let mut v = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
    v.extend_from_slice(body);
    v.extend_from_slice(b"\nendstream");
    v
}

fn open(bytes: Vec<u8>) -> Document {
    Document::open(Arc::new(bytes)).expect("opens")
}

/// The text label of each page, via its content stream, in order.
fn labels(doc: &Document) -> Vec<String> {
    pages(doc)
        .unwrap()
        .iter()
        .map(|p| {
            let page = doc.get(p.obj);
            let c = page.as_dict().and_then(|d| d.get(b"Contents").cloned());
            match c.map(|c| doc.resolve(&c)).as_deref() {
                Some(Object::Stream(s)) => {
                    let data = s.decoded().unwrap();
                    let t = String::from_utf8_lossy(&data);
                    t.split('(').nth(1).and_then(|x| x.split(')').next()).unwrap_or("").to_string()
                }
                _ => "blank".into(),
            }
        })
        .collect()
}

/// Save incrementally, verify the prefix is untouched, and reopen — both with our reader and with
/// an independent parser (hayro-syntax) that must agree on the page count.
fn save_and_reopen(doc: &Document) -> Document {
    let original = doc.bytes().clone();
    let bytes = write_incremental(doc, &SaveOptions::default()).expect("saves");
    assert_eq!(&bytes[..original.len()], &original[..], "incremental save must not modify existing bytes");
    let reopened = open(bytes.clone());
    let independent = hayro_syntax::Pdf::new(bytes).expect("hayro opens our output");
    assert_eq!(independent.pages().len(), page_count(&reopened).unwrap(), "readers agree on page count");
    reopened
}

#[test]
fn walks_nested_tree_in_order() {
    let doc = open(fixture());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3"]);
}

#[test]
fn rotate_uses_inherited_rotation_and_round_trips() {
    let mut doc = open(fixture());
    rotate_pages(&mut doc, &[0, 2], 90).unwrap();
    rotate_pages(&mut doc, &[2], -270).unwrap(); // page 3: inherited 90 + 90 - 270 = -90 ≡ 270
    let doc = save_and_reopen(&doc);
    let rot = |i: usize| {
        let p = pages(&doc).unwrap()[i].obj;
        doc.get(p).as_dict().and_then(|d| d.int(b"Rotate"))
    };
    assert_eq!(rot(0), Some(180), "inherited 90 + 90");
    assert_eq!(rot(1), None, "untouched page keeps inheriting");
    assert_eq!(rot(2), Some(270), "normalised into 0..360");
}

#[test]
fn delete_keeps_inherited_attributes_on_survivors() {
    let mut doc = open(fixture());
    delete_pages(&mut doc, &[0]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 2", "Page 3"]);
    // Page 2 inherited Resources (from the intermediate node) and MediaBox/Rotate (from the root);
    // the tree was flattened, so they must now be on the page itself.
    let p = doc.get(pages(&doc).unwrap()[0].obj);
    let d = p.as_dict().unwrap();
    assert!(d.contains(b"Resources") && d.contains(b"MediaBox"));
    assert_eq!(d.int(b"Rotate"), Some(90));
    let root = doc.get(doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Pages").unwrap());
    assert_eq!(root.as_dict().unwrap().int(b"Count"), Some(2));
}

#[test]
fn cannot_delete_every_page_or_missing_pages() {
    let mut doc = open(fixture());
    assert_eq!(delete_pages(&mut doc, &[0, 1, 2]), Err(OrganizeError::WouldRemoveAllPages));
    assert_eq!(delete_pages(&mut doc, &[7]), Err(OrganizeError::NoSuchPage(7)));
    assert_eq!(rotate_pages(&mut doc, &[3], 90), Err(OrganizeError::NoSuchPage(3)));
    assert!(!doc.is_modified(), "failed operations leave the document untouched");
}

#[test]
fn move_pages_reorders() {
    let mut doc = open(fixture());
    move_pages(&mut doc, &[2], 0).unwrap();
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2"]);
    move_pages(&mut doc, &[0, 1], 3).unwrap();
    assert_eq!(labels(&doc), ["Page 2", "Page 3", "Page 1"]);
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 2", "Page 3", "Page 1"]);
}

#[test]
fn inserts_blank_page() {
    let mut doc = open(fixture());
    insert_blank_page(&mut doc, 1, 612.0, 792.0).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 1", "blank", "Page 2", "Page 3"]);
    let hay = hayro_syntax::Pdf::new(doc.bytes().clone()).unwrap();
    let (w, h) = hay.pages().get(1).unwrap().render_dimensions();
    assert_eq!((w, h), (612.0, 792.0));
}

#[test]
fn document_info_edits_round_trip_with_unicode() {
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "Résumé — 履歴書").unwrap();
    set_info(&mut doc, "Author", "PrintCraft").unwrap();
    set_info(&mut doc, "Producer", "").unwrap(); // clearing removes the key
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Résumé — 履歴書"));
    assert_eq!(info(&doc, "Author").as_deref(), Some("PrintCraft"));
    assert_eq!(info(&doc, "Producer"), None);
}

#[test]
fn successive_incremental_saves_stack_revisions() {
    let mut doc = open(fixture());
    rotate_pages(&mut doc, &[0], 90).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(doc.revisions().len(), 2);
    let mut doc = doc;
    delete_pages(&mut doc, &[1]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(doc.revisions().len(), 3);
    assert_eq!(labels(&doc), ["Page 1", "Page 3"]);
}

#[test]
fn full_save_drops_unreachable_objects() {
    let mut doc = open(fixture());
    delete_pages(&mut doc, &[0, 1]).unwrap();
    let bytes = write_full(&doc, &SaveOptions::default()).unwrap();
    let full = open(bytes.clone());
    assert_eq!(labels(&full), ["Page 3"]);
    // Deleted pages, their content streams and the orphaned intermediate node are gone.
    assert!(full.object_numbers().len() < doc.object_numbers().len());
    assert_eq!(info(&full, "Title").as_deref(), Some("Original"));
    assert_eq!(hayro_syntax::Pdf::new(bytes).unwrap().pages().len(), 1);
}
