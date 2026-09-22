use gtk::prelude::*;
use rstpd::{
    core::{Search, SearchMode},
    editor::{CharacterCounts, Editor, Palette, sci::*},
    search_results,
    symbols::ShowSymbols,
};
use std::sync::atomic::AtomicBool;

fn representation(editor: &Editor, ch: char) -> String {
    let mut bytes = [0u8; 5];
    ch.encode_utf8(&mut bytes[..4]);
    let mut output = [0u8; 128];
    let length = unsafe {
        editor.send_raw(
            SCI_GETREPRESENTATION,
            bytes.as_ptr() as usize,
            output.as_mut_ptr() as isize,
        )
    };
    String::from_utf8(output[..length.max(0) as usize].to_vec()).unwrap()
}

#[test]
fn native_symbols_counts_and_results_preserve_document_state() {
    gtk::init().expect("Run native GTK tests with a display or xvfb-run");
    let parent = gtk::Window::new(gtk::WindowType::Toplevel);
    let panes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    parent.add(&panes);
    let left = Editor::new().unwrap();
    let right = Editor::new().unwrap();
    let map = Editor::new().unwrap();
    let results = Editor::new().unwrap();
    panes.pack_start(left.widget(), true, true, 0);
    panes.pack_start(right.widget(), true, true, 0);
    let document = left.create_document().unwrap();
    for view in [&left, &right, &map] {
        view.attach(&document);
    }
    assert_eq!(
        left.character_counts().unwrap(),
        CharacterCounts {
            total: 0,
            selected: 0
        }
    );
    let text = "A\u{e9}\u{1f680}\r\ne\u{301}\t\0\u{4e2d}";
    left.set_text(text).unwrap();
    assert_eq!(
        left.character_counts().unwrap(),
        CharacterCounts {
            total: 10,
            selected: 0
        }
    );
    left.select(3..7);
    assert_eq!(
        left.character_counts().unwrap().to_string(),
        "1 of 10 characters"
    );
    left.select(7..9);
    assert_eq!(left.character_counts().unwrap().selected, 2);
    left.send(SCI_SETSELECTION, 3, 1);
    left.send(SCI_ADDSELECTION, 7, 3);
    assert_eq!(left.character_counts().unwrap().selected, 2);
    assert_eq!(right.character_counts().unwrap().selected, 0);
    left.select(0..left.length());
    assert_eq!(left.character_counts().unwrap().selected, 10);
    left.replace(0..3, "\u{3bb}").unwrap();
    assert_eq!(right.character_counts().unwrap().total, 9);
    left.send(SCI_UNDO, 0, 0);
    assert_eq!(left.character_counts().unwrap().total, 10);
    left.send(SCI_REDO, 0, 0);
    assert_eq!(left.character_counts().unwrap().total, 9);
    left.set_text("\u{e9}x\n\u{1f680}y\n").unwrap();
    left.send(SCI_SETRECTANGULARSELECTIONANCHOR, 0, 0);
    left.send(SCI_SETRECTANGULARSELECTIONCARET, 8, 0);
    let text = left.text().unwrap();
    let selected: usize = (0..left.send(SCI_GETSELECTIONS, 0, 0) as usize)
        .map(|index| {
            let start = left.send(SCI_GETSELECTIONNSTART, index, 0) as usize;
            let end = left.send(SCI_GETSELECTIONNEND, index, 0) as usize;
            text[start..end].chars().count()
        })
        .sum();
    assert_eq!(
        left.character_counts().unwrap(),
        CharacterCounts { total: 6, selected }
    );
    left.send(SCI_SETRECTANGULARSELECTIONANCHOR, 3, 0);
    left.send(SCI_SETRECTANGULARSELECTIONCARET, 9, 0);
    left.send(SCI_SETRECTANGULARSELECTIONANCHORVIRTUALSPACE, 2, 0);
    left.send(SCI_SETRECTANGULARSELECTIONCARETVIRTUALSPACE, 5, 0);
    assert_eq!(left.character_counts().unwrap().selected, 0);

    let text = " \tspace\r\n\u{a0}\u{200b}\u{85}\u{2028}\u{2029}\0\u{1}";
    left.set_text(text).unwrap();
    left.replace(left.length()..left.length(), "edited")
        .unwrap();
    let edited = left.text().unwrap();
    left.select(2..7);
    let mut options = ShowSymbols::default();
    options.toggle_all();
    options.indent_guides = true;
    options.wrap_markers = true;
    for dark in [false, true] {
        for view in [&left, &right] {
            view.show_symbols(options, Palette::new(dark));
            assert_eq!(view.send(SCI_GETVIEWWS, 0, 0), 1);
            assert_eq!(view.send(SCI_GETVIEWEOL, 0, 0), 1);
            assert_eq!(view.send(SCI_GETINDENTATIONGUIDES, 0, 0), 3);
            assert_eq!(view.send(SCI_GETWRAPVISUALFLAGS, 0, 0), 1);
            for (ch, name) in [
                ('\0', "NUL"),
                ('\u{a0}', "NBSP"),
                ('\u{200b}', "ZWSP"),
                ('\u{85}', "NEL"),
                ('\u{2028}', "LS"),
            ] {
                assert_eq!(representation(view, ch), name);
            }
            assert_eq!(view.text().unwrap(), edited);
            assert_ne!(view.send(SCI_GETMODIFY, 0, 0), 0);
        }
    }
    assert_eq!(left.selection(), 2..7);
    assert_eq!(map.send(SCI_GETVIEWWS, 0, 0), 0);
    assert_eq!(representation(&map, '\u{a0}'), "");
    options.toggle_all();
    left.show_symbols(options, Palette::new(false));
    assert_eq!(representation(&left, '\0'), " ");
    assert_eq!(representation(&left, '\u{a0}'), "");
    assert_eq!(left.send(SCI_GETINDENTATIONGUIDES, 0, 0), 3);
    left.send(SCI_UNDO, 0, 0);
    assert_eq!(left.text().unwrap(), text);
    assert_eq!(left.send(SCI_GETMODIFY, 0, 0), 0);
    let search = Search::new("space", SearchMode::Literal, true, false).unwrap();
    let result = search_results::find_all(
        &search,
        "space".into(),
        vec![search_results::Input {
            id: 1,
            revision: 0,
            title: "Untitled 1".into(),
            text: text.into(),
            tab_width: 4,
        }],
        &AtomicBool::new(false),
    )
    .unwrap()
    .render();
    results.set_read_only_text(&result.text).unwrap();
    results.highlight(&result.highlight).unwrap();
    for range in &result.emphasis {
        results
            .indicator(search_results::MATCH_INDICATOR, range.clone())
            .unwrap();
    }
    assert_eq!(results.send(SCI_GETREADONLY, 0, 0), 1);
    results.send(SCI_CLEARALL, 0, 0);
    assert_eq!(results.text().unwrap(), result.text);
    assert_ne!(
        results.send(
            SCI_INDICATORVALUEAT,
            search_results::MATCH_INDICATOR,
            result.emphasis[0].start as isize
        ),
        0
    );
    assert_ne!(results.send(SCI_GETFOLDLEVEL, 1, 0) & 0x2000, 0);
    left.set_text(&"A\u{1f680}\r\n".repeat(5000)).unwrap();
    left.select(1..left.length() - 1);
    assert_eq!(
        left.character_counts().unwrap(),
        CharacterCounts {
            total: 20_000,
            selected: 19_998
        }
    );
    let second = left.create_document().unwrap();
    left.attach(&second);
    left.set_text("Other tab").unwrap();
    assert_eq!(left.character_counts().unwrap().total, 9);
    left.attach(&document);
    assert_eq!(left.character_counts().unwrap().total, 20_000);
    unsafe {
        parent.destroy();
    }
}

#[test]
fn counts_use_the_requested_wording() {
    assert_eq!(
        CharacterCounts {
            total: 4995,
            selected: 852
        }
        .to_string(),
        "852 of 4995 characters"
    );
    assert_eq!(
        CharacterCounts {
            total: 4995,
            selected: 0
        }
        .to_string(),
        "4995 characters"
    );
    assert_eq!(
        CharacterCounts {
            total: 1,
            selected: 1
        }
        .to_string(),
        "1 of 1 character"
    );
}
