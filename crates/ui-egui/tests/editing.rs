//! Headless UI tests for editing and saving: the organize toolbar, selection, undo/redo keys,
//! save/save-as, the unsaved-changes prompt and editable document properties.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use printcraft_render::{PageRenderer, RenderRequest, RequestKind};
use printcraft_ui_egui::{CloseRequest, PrintCraftApp};

/// An `n`-page document with a proper xref table; page `i` shows "Page i+1".
fn fixture(n: usize) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")));
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i));
        let body = format!("BT /F1 24 Tf 20 150 Td (Page {}) Tj ET", i + 1);
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn harness(pages: usize, setup: impl FnOnce(&mut PrintCraftApp) + 'static) -> Harness<'static, PrintCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PrintCraftApp::new();
        app.open_bytes("doc.pdf", None, fixture(pages)).expect("fixture opens");
        setup(&mut app);
        app
    });
    h.run_steps(4);
    h
}

fn organize(pages: usize) -> Harness<'static, PrintCraftApp> {
    harness(pages, |app| app.set_option("organize", "on").unwrap())
}

/// Page labels of the active document, read back from its current bytes.
fn page_texts(app: &PrintCraftApp) -> Vec<String> {
    let doc = app.session.get(app.views[0].id).unwrap();
    let mut r = PageRenderer::new(doc.bytes.clone(), Default::default());
    (0..r.page_count())
        .map(|p| {
            let out = r.render(RenderRequest { page: p, kind: RequestKind::Text, scale: 1.0, ..Default::default() });
            out.text.map(|t| t.plain_text().trim().to_string()).unwrap_or_default()
        })
        .collect()
}

fn dirty(h: &Harness<'static, PrintCraftApp>) -> bool {
    let app = h.state();
    app.session.get(app.views[0].id).is_some_and(|d| d.dirty)
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("printcraft-ui-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn organize_delete_then_undo_and_redo_with_keys() {
    let mut h = organize(3);
    h.get_by_label("Page 2").click();
    h.run_steps(2);
    h.get_by_label_contains("1 page selected");
    h.get_by_label("Delete pages (Delete)").click();
    h.run_steps(3);
    assert_eq!(page_texts(h.state()), ["Page 1", "Page 3"]);
    assert!(dirty(&h));
    h.get_by_label("doc.pdf (edited)"); // the tab shows unsaved changes

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(page_texts(h.state()), ["Page 1", "Page 2", "Page 3"]);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(3);
    assert_eq!(page_texts(h.state()), ["Page 1", "Page 3"]);
}

#[test]
fn shift_click_selects_a_range_and_moves_it() {
    let mut h = organize(4);
    h.get_by_label("Page 1").click();
    h.run_steps(1);
    h.get_by_label("Page 2").click_modifiers(Modifiers::SHIFT);
    h.run_steps(2);
    h.get_by_label_contains("2 pages selected");
    h.get_by_label("Move later").click();
    h.run_steps(3);
    assert_eq!(page_texts(h.state()), ["Page 3", "Page 1", "Page 2", "Page 4"]);
    // The moved pages stay selected at their new position, so repeated clicks keep moving them.
    assert_eq!(h.state().views[0].selected.iter().copied().collect::<Vec<_>>(), [1, 2]);
    h.get_by_label("Move later").click();
    h.run_steps(3);
    assert_eq!(page_texts(h.state()), ["Page 3", "Page 4", "Page 1", "Page 2"]);
}

#[test]
fn command_click_toggles_and_rotation_applies_to_selection() {
    let mut h = organize(3);
    h.get_by_label("Page 1").click();
    h.run_steps(1);
    h.get_by_label("Page 3").click_modifiers(Modifiers::COMMAND);
    h.run_steps(2);
    h.get_by_label("Rotate clockwise").click();
    h.run_steps(3);
    let rot: Vec<u16> = {
        let app = h.state();
        app.session.get(app.views[0].id).unwrap().info.pages.iter().map(|p| p.rotation).collect()
    };
    assert_eq!(rot, [90, 0, 90]);
}

#[test]
fn delete_is_refused_when_it_would_remove_every_page() {
    let mut h = organize(2);
    h.state_mut().views[0].select_pages(&[0, 1]);
    h.run_steps(2);
    h.key_press(Key::Delete);
    h.run_steps(3);
    assert_eq!(page_texts(h.state()).len(), 2, "a document keeps at least one page");
    assert!(!dirty(&h));
}

#[test]
fn insert_blank_page_after_selection() {
    let mut h = organize(2);
    h.get_by_label("Page 1").click();
    h.run_steps(1);
    h.get_by_label("Insert a blank page after the selection").click();
    h.run_steps(3);
    assert_eq!(page_texts(h.state()), ["Page 1", "", "Page 2"]);
    let app = h.state();
    let info = &app.session.get(app.views[0].id).unwrap().info;
    assert_eq!((info.pages[1].width, info.pages[1].height), (200.0, 300.0), "matches the neighbouring page");
}

#[test]
fn save_writes_an_incremental_update_and_clears_dirty() {
    let out = temp_path("saved.pdf");
    let original = fixture(3);
    let target = out.to_string_lossy().into_owned();
    let mut h = organize(3);
    h.state_mut().save_override = Some(target.clone());
    h.get_by_label("Page 3").click();
    h.run_steps(1);
    h.get_by_label("Rotate counterclockwise").click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    let saved = std::fs::read(&out).expect("file written");
    assert_eq!(&saved[..original.len()], &original[..], "original bytes untouched");
    assert!(!dirty(&h));
    h.get_by_label("saved.pdf"); // the tab takes the new name, no edited marker
    // What was written is what the app now shows.
    assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().bytes.as_slice(), saved.as_slice());
    let reopened = printcraft_render::inspect(std::sync::Arc::new(saved), None).unwrap();
    assert_eq!(reopened.pages[2].rotation, 270);
    let _ = std::fs::remove_file(out);
}

#[test]
fn closing_a_dirty_tab_asks_and_cancel_keeps_it() {
    let mut h = organize(2);
    h.get_by_label("Rotate clockwise").click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(3);
    h.get_by_label_contains("Save changes to “doc.pdf”");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().views.len(), 1, "cancel keeps the document open");
    assert!(dirty(&h));
    assert!(h.state().close_request.is_none());

    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(3);
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    assert!(h.state().views.is_empty(), "discarding closes the tab");
}

#[test]
fn closing_a_dirty_tab_can_save_first() {
    let out = temp_path("closed.pdf");
    let mut h = organize(2);
    h.state_mut().save_override = Some(out.to_string_lossy().into_owned());
    h.get_by_label("Rotate clockwise").click();
    h.run_steps(3);
    h.state_mut().request_close_tab(0);
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert!(h.state().views.is_empty());
    let saved = printcraft_render::inspect(std::sync::Arc::new(std::fs::read(&out).unwrap()), None).unwrap();
    assert_eq!(saved.pages[0].rotation, 90);
    let _ = std::fs::remove_file(out);
}

#[test]
fn clean_tabs_close_without_asking() {
    let mut h = harness(1, |_| {});
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(3);
    assert!(h.state().views.is_empty());
    assert!(h.query_by_label("Don't save").is_none());
}

#[test]
fn quitting_with_unsaved_changes_asks_for_each_document() {
    let mut h = harness(2, |app| {
        app.open_bytes("second.pdf", None, fixture(1)).unwrap();
    });
    // Edit both documents.
    for tab in 0..2 {
        h.state_mut().active = Some(tab);
        h.state_mut().views[tab].select_pages(&[0]);
        h.state_mut().apply_edit(printcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 });
    }
    h.state_mut().close_request = Some(CloseRequest::Quit);
    h.run_steps(3);
    h.get_by_label_contains("Save changes to “doc.pdf”");
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    h.get_by_label_contains("Save changes to “second.pdf”");
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    assert!(h.state().views.is_empty());
    assert!(h.state().close_request.is_none());
}

#[test]
fn document_properties_edit_is_one_undoable_step() {
    let mut h = harness(1, |app| app.set_option("dialog", "properties").unwrap());
    let title = h.get_by_role_and_label(Role::TextInput, "Title");
    title.focus();
    title.type_text("Annual report");
    h.run_steps(2);
    let author = h.get_by_role_and_label(Role::TextInput, "Author");
    author.focus();
    author.type_text("Finance team");
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!(doc.info.title.as_deref(), Some("Annual report"), "inspection sees the new title");
    assert_eq!(doc.info_value("Author").as_deref(), Some("Finance team"));
    assert_eq!(doc.can_undo(), Some("Change document properties"));
    assert!(app.dialog.is_none());

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!(doc.info.title, None);
    assert_eq!(doc.info_value("Author"), None);
}

#[test]
fn cancelling_properties_discards_the_draft() {
    let mut h = harness(1, |app| app.set_option("dialog", "properties").unwrap());
    let title = h.get_by_role_and_label(Role::TextInput, "Title");
    title.focus();
    title.type_text("Draft");
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(!dirty(&h));
    assert!(h.state().props_draft.is_none());
}

#[test]
fn edit_menu_names_the_step_to_undo() {
    let mut h = harness(2, |app| {
        app.apply_edit(printcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 });
    });
    h.get_by_label("Menu").click();
    h.run_steps(2);
    h.get_by_label("Edit ⏵").hover(); // submenus open on hover; their labels carry the arrow
    h.run_steps(3);
    h.get_by_label_contains("Undo Rotate page").click(); // the label includes the shortcut
    h.run_steps(3);
    let app = h.state();
    let doc = app.session.get(app.views[0].id).unwrap();
    assert_eq!(doc.can_redo(), Some("Rotate page"));
    assert_eq!(doc.info.pages[0].rotation, 0);
}
