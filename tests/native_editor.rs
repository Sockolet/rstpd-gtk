use gtk::prelude::*;
use rstpd::{
    core::{EditorFont, MAX_DOCUMENT_BYTES},
    editor::{self, Editor, Palette, sci::*},
    languages,
};
use std::{
    cell::RefCell,
    ffi::{CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

unsafe extern "C" {
    fn rstpd_test_emit_uri_notification(widget: *mut gtk::ffi::GtkWidget, text: *const c_char);
}

fn emit_uri_drop(editor: &Editor, bytes: &[u8]) {
    let payload = CString::new(bytes).unwrap();
    unsafe {
        rstpd_test_emit_uri_notification(editor.widget().as_ptr(), payload.as_ptr());
    }
}

fn style_font(editor: &Editor, style: usize) -> String {
    let size = unsafe { editor.send_raw(SCI_STYLEGETFONT, style, 0) } as usize;
    let mut bytes = vec![0; size + 1];
    unsafe {
        editor.send_raw(SCI_STYLEGETFONT, style, bytes.as_mut_ptr() as isize);
    }
    bytes.truncate(size);
    String::from_utf8(bytes).unwrap()
}

#[test]
fn native_editing_unicode_split_selection_highlighting_and_undo() {
    unsafe {
        assert!(
            Editor::new().is_err(),
            "Initialize GTK before creating an editor."
        );
        gtk::init().expect("GTK3 requires a display; run this test under xvfb-run when headless.");
        let unparented = Editor::new().unwrap();
        let weak = unparented.widget().downgrade();
        let owned_clone = unparented.clone();
        drop(unparented);
        owned_clone.set_text("A clone owns its widget.").unwrap();
        assert_eq!(owned_clone.text().unwrap(), "A clone owns its widget.");
        drop(owned_clone);
        assert!(
            weak.upgrade().is_none(),
            "Unparented widgets must not leak."
        );

        let parent = gtk::Window::new(gtk::WindowType::Toplevel);
        parent.set_default_size(800, 600);
        let panes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        parent.add(&panes);
        let left = Editor::new().unwrap();
        let right = Editor::new().unwrap();
        let left_clone = left.clone();
        let notifications = Rc::new(RefCell::new(Vec::new()));
        let callback_lifetime = Rc::downgrade(&notifications);
        let received = Rc::clone(&notifications);
        left.connect_notify(move |notification| received.borrow_mut().push(notification));
        panes.pack_start(left.widget(), true, true, 0);
        panes.pack_start(right.widget(), true, true, 0);
        parent.show_all();
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        let document = left.create_document().unwrap();
        left.attach(&document);
        right.attach(&document);
        left.set_text("caf\u{e9}\0\u{1f680}\r\nsecond\r\n").unwrap();
        assert_eq!(left.text().unwrap(), "caf\u{e9}\0\u{1f680}\r\nsecond\r\n");
        assert_eq!(left.range(3..5).unwrap(), "\u{e9}");
        assert_eq!(left.range(5..6).unwrap(), "\0");
        assert_eq!(left.range(6..10).unwrap(), "\u{1f680}");
        assert!(left.range(4..5).is_err());
        assert!(left.replace(0..4, "bad").is_err());
        assert!(left.replace(100..101, "bad").is_err());
        assert_eq!(left.send(SCI_GETCODEPAGE, 0, 0), 65001);
        assert_eq!(left.send(SCI_POSITIONAFTER, 3, 0), 5);
        assert_eq!(left.send(SCI_POSITIONAFTER, 6, 0), 10);
        assert!(catch_unwind(AssertUnwindSafe(|| left.send(SCI_SETTEXT, 0, 0))).is_err());
        assert!(notifications.borrow().iter().any(|notification| {
            notification.code == SCN_MODIFIED
                && notification.position == 0
                && notification.modification & 1 != 0
        }));
        let dropped_uris =
            "# Local files\r\nfile:///tmp/caf\u{e9}.txt\r\nfile:///tmp/with%20space.txt\r\n";
        emit_uri_drop(&left, dropped_uris.as_bytes());
        let uri_notification = notifications
            .borrow()
            .iter()
            .find(|notification| notification.code == SCN_URIDROPPED)
            .cloned()
            .expect("GTK file drops must produce a Scintilla URI notification.");
        assert_eq!(uri_notification.text.as_deref(), Some(dropped_uris));
        assert!(
            notifications
                .borrow()
                .iter()
                .filter(|notification| notification.code != SCN_URIDROPPED)
                .all(|notification| notification.text.is_none())
        );
        let mut limit_payload = vec![b'#'; MAX_DOCUMENT_BYTES];
        emit_uri_drop(&left, &limit_payload);
        {
            let mut received = notifications.borrow_mut();
            let index = received
                .iter()
                .rposition(|notification| notification.code == SCN_URIDROPPED)
                .unwrap();
            let copied = received.remove(index).text.unwrap();
            assert_eq!(copied.len(), MAX_DOCUMENT_BYTES);
            assert!(copied.bytes().all(|byte| byte == b'#'));
        }
        let uri_count = || {
            notifications
                .borrow()
                .iter()
                .filter(|notification| notification.code == SCN_URIDROPPED)
                .count()
        };
        limit_payload.push(b'#');
        for invalid in [limit_payload.as_slice(), &[0xff]] {
            let before = uri_count();
            emit_uri_drop(&left, invalid);
            assert_eq!(
                uri_count(),
                before,
                "Invalid URI payloads must not be queued."
            );
        }
        drop(limit_payload);
        assert_eq!(left.text().unwrap(), "caf\u{e9}\0\u{1f680}\r\nsecond\r\n");
        left.replace(0..5, "tea").unwrap();
        assert_eq!(right.text().unwrap(), "tea\0\u{1f680}\r\nsecond\r\n");
        right.send(SCI_UNDO, 0, 0);
        assert!(left.text().unwrap().starts_with("caf\u{e9}"));

        left.set_text("one\ntwo\n").unwrap();
        left.send(SCI_SETSELECTION, 3, 0);
        left.send(SCI_ADDSELECTION, 7, 4);
        assert_eq!(left.send(SCI_GETSELECTIONS, 0, 0), 2);
        left.transform_selections(str::to_uppercase).unwrap();
        assert_eq!(right.text().unwrap(), "ONE\nTWO\n");
        assert!(notifications.borrow().iter().any(|notification| {
            notification.code == SCN_MODIFIED
                && notification.position == 4
                && notification.modification & 1 != 0
        }));
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        assert!(notifications.borrow().iter().any(|notification| {
            notification.code == SCN_UPDATEUI && notification.updated & 1 != 0
        }));
        left.send(SCI_UNDO, 0, 0);
        assert_eq!(left.text().unwrap(), "one\ntwo\n");
        left.send(SCI_SETRECTANGULARSELECTIONANCHOR, 1, 0);
        left.send(SCI_SETRECTANGULARSELECTIONCARET, 6, 0);
        assert_eq!(left.send(SCI_SELECTIONISRECTANGLE, 0, 0), 1);
        assert_eq!(left.send(SCI_GETSELECTIONS, 0, 0), 2);
        left.transform_selections(str::to_uppercase).unwrap();
        assert_eq!(left.text().unwrap(), "oNe\ntWo\n");
        left.send(SCI_UNDO, 0, 0);
        assert_eq!(left.text().unwrap(), "one\ntwo\n");

        let available = editor::available_lexers();
        let catalog = languages::catalog(&available);
        for language in &catalog {
            left.language(language, Palette::new(false)).unwrap();
            if language.uses_container() {
                assert_eq!(left.send(SCI_GETLEXER, 0, 0), 0);
                continue;
            }
            let len = left.send_raw(SCI_GETLEXERLANGUAGE, 0, 0) as usize;
            let mut name = vec![0u8; len + 1];
            left.send_raw(SCI_GETLEXERLANGUAGE, 0, name.as_mut_ptr() as isize);
            assert!(
                name[..len].eq_ignore_ascii_case(language.lexer.as_bytes()),
                "{}",
                language.name
            );
        }
        let rust = &catalog[languages::detect(std::path::Path::new("main.rs"), &catalog)];
        let selected_font = EditorFont::new("Sans", 1250).unwrap();
        left.language_with_font(rust, Palette::new(true), &selected_font)
            .unwrap();
        for style in [0, 1, 32, 33, 38] {
            assert_eq!(style_font(&left, style), "Sans");
            assert_eq!(left.send(SCI_STYLEGETSIZEFRACTIONAL, style, 0), 1250);
        }
        let large = EditorFont::new("Monospace", 3200).unwrap();
        left.theme_with_font(rust, Palette::new(false), &large);
        let digits = CString::new("9999").unwrap();
        let number_width = left.send_raw(SCI_TEXTWIDTH, 33, digits.as_ptr() as isize);
        assert!(
            left.send(SCI_GETMARGINWIDTHN, 0, 0) >= number_width + 12,
            "Line numbers must fit the selected font size"
        );
        left.theme_with_font(rust, Palette::new(true), &selected_font);
        assert_eq!(style_font(&left, 32), "Sans");
        assert_eq!(left.send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 1250);
        left.language(rust, Palette::new(true)).unwrap();
        assert_eq!(style_font(&left, 32), EditorFont::default().family());
        assert_eq!(left.send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 1100);
        left.set_text("fn main() { let x = 123; } // comment")
            .unwrap();
        left.send(SCI_COLOURISE, 0, -1);
        assert_ne!(
            left.send(SCI_GETSTYLEAT, 0, 0),
            left.send(SCI_GETSTYLEAT, 2, 0)
        );
        assert_ne!(
            left.send(SCI_GETSTYLEAT, 0, 0),
            left.send(SCI_GETSTYLEAT, 19, 0)
        );
        let position = 12;
        left.send(SCI_GOTOPOS, position, 0);
        left.attach(&document);
        assert_eq!(left.position(), position);

        left.set_text("let text = String::from(\"hello\");\ntext.tr")
            .unwrap();
        left.send(SCI_GOTOPOS, left.length(), 0);
        left.complete(rust, &[], true).unwrap();
        assert_ne!(left.send(SCI_AUTOCACTIVE, 0, 0), 0);
        left.send(SCI_AUTOCCANCEL, 0, 0);
        left.set_text(include_str!("fixtures/functions.rs"))
            .unwrap();
        left.send(SCI_GOTOPOS, left.length(), 0);
        left.call_tip(rust, &[]).unwrap();
        assert_ne!(left.send(SCI_CALLTIPACTIVE, 0, 0), 0);
        left.send(SCI_CALLTIPCANCEL, 0, 0);

        let definition = rstpd::udl::import(include_str!("fixtures/custom-language.xml"))
            .unwrap()
            .remove(0);
        let mut custom_catalog = languages::catalog(&available);
        let custom = languages::add_custom(&mut custom_catalog, definition.clone()).unwrap();
        left.language(&custom_catalog[custom], Palette::new(false))
            .unwrap();
        let text = include_str!("fixtures/sample.rstlang");
        left.set_text(text).unwrap();
        left.highlight(&rstpd::udl::highlight(&definition, text).unwrap())
            .unwrap();
        assert_eq!(left.send(SCI_GETSTYLEAT, text.find("say").unwrap(), 0), 4);
        assert_ne!(left.send(SCI_GETFOLDLEVEL, 0, 0) & 0x2000, 0);
        left.theme_with_font(&custom_catalog[custom], Palette::new(false), &selected_font);
        assert_eq!(style_font(&left, 4), "Sans");
        assert_eq!(left.send(SCI_STYLEGETSIZEFRACTIONAL, 4, 0), 1250);
        assert_ne!(
            left.send(SCI_STYLEGETBOLD, 4, 0),
            0,
            "Syntax font attributes must survive"
        );
        let mut custom_fonts = definition.clone();
        custom_fonts.styles[0].font = Some("Serif".into());
        custom_fonts.styles[0].font_size = Some(15);
        custom_fonts.styles[4].font = Some("Monospace".into());
        custom_fonts.styles[4].font_size = Some(18);
        let custom = languages::add_custom(&mut custom_catalog, custom_fonts).unwrap();
        left.theme_with_font(&custom_catalog[custom], Palette::new(true), &selected_font);
        assert_eq!(style_font(&left, 32), "Serif");
        assert_eq!(left.send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 1500);
        assert_eq!(style_font(&left, 4), "Monospace");
        assert_eq!(left.send(SCI_STYLEGETSIZEFRACTIONAL, 4, 0), 1800);
        assert_ne!(left.send(SCI_STYLEGETBOLD, 4, 0), 0);
        left.indicator(20, 0..5).unwrap();
        assert_ne!(left.send(SCI_INDICATORVALUEAT, 20, 1), 0);
        left.clear_diff();
        assert_eq!(left.send(SCI_INDICATORVALUEAT, 20, 1), 0);
        let text_before_padding = left.text().unwrap();
        let modified_before_padding = left.send(SCI_GETMODIFY, 0, 0);
        assert_eq!(
            left.apply_compare_padding(&[
                rstpd::comparison::Padding {
                    before: 0,
                    count: 2
                },
                rstpd::comparison::Padding {
                    before: 1,
                    count: 3
                },
            ])
            .unwrap(),
            2
        );
        assert_eq!(left.send(SCI_ANNOTATIONGETLINES, 0, 0), 3);
        assert_eq!(left.text().unwrap(), text_before_padding);
        assert_eq!(left.send(SCI_GETMODIFY, 0, 0), modified_before_padding);
        left.send(SCI_MARKERADD, 0, 22);
        assert_ne!(left.send(SCI_MARKERGET, 0, 0) & (1 << 22), 0);
        left.clear_diff();
        assert_eq!(left.send(SCI_ANNOTATIONGETLINES, 0, 0), 0);
        assert_eq!(left.send(SCI_MARKERGET, 0, 0) & (1 << 22), 0);
        left.clear_styles();
        assert_eq!(left.send(SCI_GETENDSTYLED, 0, 0) as usize, left.length());
        assert_eq!(left.send(SCI_GETSTYLEAT, text.find("say").unwrap(), 0), 0);
        parent.destroy();
        assert!(catch_unwind(AssertUnwindSafe(|| left.length())).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| left_clone.length())).is_err());
        drop(left_clone);
        drop(left);
        drop(right);
        drop(notifications);
        assert!(
            callback_lifetime.upgrade().is_none(),
            "Notification callbacks must be released."
        );
        let replacement = Editor::new().unwrap();
        replacement.attach(&document);
        assert_eq!(replacement.text().unwrap(), text);
        drop(document);
        assert_eq!(replacement.text().unwrap(), text);
        assert_eq!(uri_notification.text.as_deref(), Some(dropped_uris));
    }
}
