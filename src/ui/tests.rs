use super::*;

struct DirectoryCleanup(PathBuf);

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "Could not remove test directory {}: {error}",
                self.0.display()
            );
        }
    }
}

fn pump(app: &mut App) {
    for _ in 0..32 {
        if !gtk::events_pending() {
            break;
        }
        gtk::main_iteration_do(false);
    }
    for iteration in 0..4096 {
        let event = EVENTS.with(|events| events.borrow_mut().pop_front());
        let Some(event) = event else {
            break;
        };
        app.event(event).unwrap();
        assert!(iteration < 4095, "GTK events did not settle");
    }
    app.tick().unwrap();
}

fn wait_for(app: &mut App, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        pump(app);
        if ready(app) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Background operation timed out: {}",
            app.note
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn text(app: &mut App, value: &str) {
    app.editor().set_text(value).unwrap();
    pump(app);
}

fn contrast(foreground: u32, background: u32) -> f64 {
    let luminance = |color: u32| {
        [0.2126, 0.7152, 0.0722]
            .iter()
            .enumerate()
            .map(|(index, weight)| {
                let value = ((color >> (index * 8)) & 255) as f64 / 255.0;
                weight
                    * if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f64>()
    };
    let (a, b) = (luminance(foreground), luminance(background));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
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

fn respond_to_font_dialog(
    app: &mut App,
    selection: gtk::pango::FontDescription,
    response: gtk::ResponseType,
) -> Result<()> {
    let initial = app.editor_font.clone();
    let observed = Rc::new(RefCell::new(None));
    let capture = observed.clone();
    let timer = glib::timeout_add_local(Duration::from_millis(20), move || {
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|window| window.downcast::<gtk::FontChooserDialog>().ok());
        let Some(dialog) = dialog else {
            return glib::ControlFlow::Continue;
        };
        *capture.borrow_mut() = Some((dialog.level(), dialog.font_desc()));
        dialog.set_font_desc(&selection);
        dialog.response(response);
        glib::ControlFlow::Break
    });
    let result = app.command(EDITOR_FONT);
    if observed.borrow().is_none() {
        timer.remove();
    }
    let (level, description) = observed
        .borrow_mut()
        .take()
        .expect("The font command must open GTK's chooser");
    assert_eq!(
        level,
        gtk::FontChooserLevel::FAMILY | gtk::FontChooserLevel::SIZE
    );
    assert_eq!(
        editor_font_from_description(&description.unwrap()).unwrap(),
        initial
    );
    result
}

fn exercise_editor_font_selection(app: &mut App) {
    assert_eq!(app.editor_font, EditorFont::default());
    text(app, "font sample\nsecond line\n");
    app.editor().replace(0..0, "x").unwrap();
    app.editor().select(3..8);
    app.command(SPLIT).unwrap();
    app.command(MAP).unwrap();
    app.editors[0].send(SCI_SETZOOM, 4, 0);
    app.editors[1].send(SCI_SETZOOM, 3, 0);
    pump(app);
    let original = serde_json::to_value(app.snapshot().unwrap().documents).unwrap();
    let original_revision = app.revision;
    let document_revision = app.documents[app.index()].revision;
    let selection = app.editor().selection();
    let caret = app.editor().position();
    let undo = app.editor().send(SCI_CANUNDO, 0, 0);
    let chrome_font = gtk::Settings::default()
        .unwrap()
        .property::<Option<String>>("gtk-font-name");
    let font = EditorFont::new("Sans", 1250).unwrap();
    respond_to_font_dialog(
        app,
        editor_font_description(&font),
        gtk::ResponseType::Cancel,
    )
    .unwrap();
    pump(app);
    assert_eq!(app.editor_font, EditorFont::default());
    assert_eq!(
        app.revision, original_revision,
        "Cancelling must not modify session settings"
    );
    assert_eq!(app.editors[0].send(SCI_GETZOOM, 0, 0), 4);
    assert_eq!(app.editors[1].send(SCI_GETZOOM, 0, 0), 3);
    assert_eq!(
        serde_json::to_value(app.snapshot().unwrap().documents).unwrap(),
        original
    );

    respond_to_font_dialog(app, editor_font_description(&font), gtk::ResponseType::Ok).unwrap();
    pump(app);
    assert_eq!(app.editor_font, font);
    assert_eq!(app.documents[app.index()].revision, document_revision);
    assert_eq!(
        serde_json::to_value(app.snapshot().unwrap().documents).unwrap(),
        original,
        "Changing the font must not change text, document metadata, dirty flags or caret"
    );
    assert_eq!(app.editor().selection(), selection);
    assert_eq!(app.editor().position(), caret);
    assert_eq!(app.editor().send(SCI_CANUNDO, 0, 0), undo);
    for editor in &app.editors {
        assert_eq!(editor.send(SCI_GETZOOM, 0, 0), 0);
        for style in [0, 1, 32, 33, 38] {
            assert_eq!(style_font(editor, style), "Sans");
            assert_eq!(editor.send(SCI_STYLEGETSIZEFRACTIONAL, style, 0), 1250);
        }
    }
    assert_eq!(style_font(&app.map, 32), "Sans");
    assert_eq!(app.map.send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 200);
    assert_eq!(app.map.send(SCI_GETMARGINWIDTHN, 0, 0), 0);
    assert_eq!(
        gtk::Settings::default()
            .unwrap()
            .property::<Option<String>>("gtk-font-name"),
        chrome_font
    );
    app.editors[0].send(SCI_SETZOOM, 5, 0);
    app.command(ZOOM_RESET).unwrap();
    pump(app);
    assert_eq!(app.editor().send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 1250);
    assert_eq!(app.editor().send(SCI_GETZOOM, 0, 0), 0);
    assert_eq!(app.snapshot().unwrap().editor_font, font);

    let mut invalid = editor_font_description(&font);
    invalid.set_size(73 * gtk::pango::SCALE);
    assert!(editor_font_from_description(&invalid).is_err());
    respond_to_font_dialog(app, invalid, gtk::ResponseType::Ok).unwrap_err();
    assert_eq!(app.editor_font, font);
    let mut pixels = editor_font_description(&font);
    pixels.set_absolute_size(12.5 * f64::from(gtk::pango::SCALE));
    assert!(editor_font_from_description(&pixels).is_err());

    app.command(MAP).unwrap();
    app.command(SPLIT).unwrap();
    pump(app);
}

fn exercise_symbols_results_and_counts(app: &mut App) {
    text(app, "A\u{e9}\u{1f680}\r\ne\u{301}\t\0\u{4e2d}");
    assert!(app.status.text().contains("|   10 characters   |"));
    app.editor().select(3..7);
    pump(app);
    assert!(app.status.text().contains("1 of 10 characters"));
    let initial = app.editor().text().unwrap();
    let before = (
        app.editor().send(SCI_GETMODIFY, 0, 0),
        app.editor().send(SCI_CANUNDO, 0, 0),
    );
    let all = app
        .symbol_items
        .iter()
        .find(|(id, _)| *id == SYMBOL_ALL)
        .unwrap()
        .1
        .clone();
    all.activate();
    pump(app);
    assert!(app.show_symbols.all_characters());
    app.command(SYMBOL_INDENT).unwrap();
    app.command(SYMBOL_WRAP).unwrap();
    app.command(SPLIT).unwrap();
    pump(app);
    for theme in [THEME_LIGHT, THEME_DARK] {
        app.command(theme).unwrap();
        pump(app);
        for editor in &app.editors {
            assert_eq!(editor.send(SCI_GETVIEWWS, 0, 0), 1);
            assert_eq!(editor.send(SCI_GETVIEWEOL, 0, 0), 1);
            assert_eq!(editor.send(SCI_GETINDENTATIONGUIDES, 0, 0), 3);
            assert_eq!(editor.send(SCI_GETWRAPVISUALFLAGS, 0, 0), 1);
            assert_eq!(editor.text().unwrap(), initial);
        }
    }
    assert_eq!(
        (
            app.editor().send(SCI_GETMODIFY, 0, 0),
            app.editor().send(SCI_CANUNDO, 0, 0)
        ),
        before
    );
    assert_eq!(app.map.send(SCI_GETVIEWWS, 0, 0), 0);
    app.editors[0].select(3..7);
    app.editors[1].select(0..3);
    app.event(Event::OtherPane).unwrap();
    pump(app);
    assert!(app.status.text().contains("2 of 10 characters"));
    app.event(Event::OtherPane).unwrap();
    pump(app);
    assert!(app.status.text().contains("1 of 10 characters"));
    app.command(SYMBOL_EOL).unwrap();
    assert!(
        !app.symbol_items
            .iter()
            .find(|(id, _)| *id == SYMBOL_ALL)
            .unwrap()
            .1
            .is_active()
    );
    app.command(SYMBOL_ALL).unwrap();
    assert!(
        app.symbol_items
            .iter()
            .find(|(id, _)| *id == SYMBOL_ALL)
            .unwrap()
            .1
            .is_active()
    );
    app.command(SPLIT).unwrap();
    pump(app);
    let first = app.index();
    let first_id = app.documents[first].snapshot.id;
    text(app, "\u{e9} alpha\r\nalpha\n");
    app.new_document().unwrap();
    assert_eq!(app.editor().send(SCI_GETVIEWWS, 0, 0), 1);
    assert!(app.status.text().contains("0 characters"));
    text(app, "alpha beta alpha");
    let second = app.index();
    let second_id = app.documents[second].snapshot.id;
    app.command(FIND).unwrap();
    app.search.mode.set_active(Some(0));
    app.search.query.set_text("alpha");
    assert!(app.search.query.has_frame() && app.search.replace.has_frame());
    for (_, button) in &app.search.buttons {
        assert_eq!(button.relief(), gtk::ReliefStyle::Normal);
    }
    for (_, button) in &app.results.buttons {
        assert_eq!(button.relief(), gtk::ReliefStyle::Normal);
    }
    for size in [(780, 600), (1280, 840)] {
        app.window.resize(size.0, size.1);
        wait_for(app, |app| app.search.query.allocated_width() > 100);
        let controls: Vec<gtk::Widget> = vec![
            app.search.query.clone().upcast(),
            app.search.replace.clone().upcast(),
            app.search.mode.clone().upcast(),
            app.search.case.clone().upcast(),
            app.search.word.clone().upcast(),
        ]
        .into_iter()
        .chain(
            app.search
                .buttons
                .iter()
                .map(|(_, button)| button.clone().upcast()),
        )
        .collect();
        for (i, control) in controls.iter().enumerate() {
            let (x, y) = control
                .translate_coordinates(&app.search.container, 0, 0)
                .unwrap();
            assert!(x >= 0 && y >= 0);
            assert!(x + control.allocated_width() <= app.search.container.allocated_width());
            for other in &controls[i + 1..] {
                let (ox, oy) = other
                    .translate_coordinates(&app.search.container, 0, 0)
                    .unwrap();
                assert!(
                    !(x < ox + other.allocated_width()
                        && ox < x + control.allocated_width()
                        && y < oy + other.allocated_height()
                        && oy < y + control.allocated_height()),
                    "Search controls overlap"
                );
            }
        }
    }
    app.search
        .buttons
        .iter()
        .find(|(id, _)| *id == FIND_ALL_CURRENT)
        .unwrap()
        .1
        .emit_clicked();
    wait_for(app, |app| {
        app.results.job.is_none() && app.results.data.is_some()
    });
    assert_eq!(app.results.data.as_ref().unwrap().count(), 2);
    assert!(app.results.container.is_visible());
    assert_eq!(app.results.editor.send(SCI_GETREADONLY, 0, 0), 1);
    app.command(FIND_ALL_OPEN).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.results.data.as_ref().unwrap().count(), 4);
    assert_eq!(app.results.data.as_ref().unwrap().files.len(), 2);
    assert_eq!(app.results.links.len(), 4);
    app.command(RESULTS_NEXT).unwrap();
    pump(app);
    assert_eq!(app.documents[app.index()].snapshot.id, first_id);
    assert_eq!(app.editor().selection(), 3..8);
    assert!(app.status.text().contains("5 of "));
    app.command(RESULTS_NEXT).unwrap();
    assert_eq!(
        app.editor().range(app.editor().selection()).unwrap(),
        "alpha"
    );
    app.command(RESULTS_NEXT).unwrap();
    pump(app);
    assert_eq!(app.documents[app.index()].snapshot.id, second_id);
    assert_eq!(app.editor().selection(), 0..5);
    app.command(RESULTS_PREVIOUS).unwrap();
    pump(app);
    assert_eq!(app.documents[app.index()].snapshot.id, first_id);
    let row = app.results.links[2].row;
    let position = app.results.editor.send(SCI_POSITIONFROMLINE, row, 0) as usize;
    app.event(Event::ResultActivate(position)).unwrap();
    pump(app);
    assert_eq!(app.documents[app.index()].snapshot.id, second_id);
    let divider = app.content.position();
    app.content.set_position(divider.saturating_sub(40));
    pump(app);
    assert_ne!(app.content.position(), divider);
    for theme in [THEME_LIGHT, THEME_DARK] {
        app.command(theme).unwrap();
        pump(app);
        assert_eq!(app.results.editor.send(SCI_GETREADONLY, 0, 0), 1);
        assert_eq!(
            app.results.editor.send(SCI_STYLEGETBACK, 32, 0),
            app.palette.background as isize
        );
    }
    app.editor()
        .select(app.editor().length()..app.editor().length());
    app.editor()
        .replace(app.editor().length()..app.editor().length(), "!")
        .unwrap();
    pump(app);
    let caret = app.editor().position();
    app.event(Event::ResultActivate(position)).unwrap();
    assert!(app.note.contains("changed since the search"));
    assert_eq!(app.editor().position(), caret);
    app.command(RESULTS_TOGGLE).unwrap();
    app.command(RESULTS_TOGGLE).unwrap();
    pump(app);
    assert!(
        app.results.has_focus(),
        "Results must receive keyboard focus"
    );
    let old_text = app.editor().text().unwrap();
    app.command(PASTE).unwrap();
    assert_eq!(app.editor().text().unwrap(), old_text);
    app.command(SELECT_ALL).unwrap();
    assert_eq!(
        app.results.editor.selection(),
        0..app.results.editor.length()
    );
    app.command(RESULTS_CLOSE).unwrap();
    pump(app);
    assert!(!app.results.container.is_visible());
    app.command(UNDO).unwrap();
    pump(app);
    app.command(FIND_ALL_OPEN).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    app.results
        .editor
        .send(SCI_GOTOLINE, app.results.links[2].row, 0);
    app.command(RESULTS_ACTIVATE).unwrap();
    pump(app);
    assert_eq!(app.documents[app.index()].snapshot.id, second_id);
    app.search.query.set_text("missing-text");
    app.command(FIND_ALL_CURRENT).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.results.data.as_ref().unwrap().count(), 0);
    app.command(FIND_ALL_OPEN).unwrap();
    app.command(RESULTS_CANCEL).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.note, "Search cancelled.");
    app.search.query.set_text("^");
    app.search.mode.set_active(Some(2));
    app.new_document().unwrap();
    pump(app);
    let empty = app.index();
    app.command(FIND_ALL_CURRENT).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.results.data.as_ref().unwrap().count(), 1);
    app.command(RESULTS_NEXT).unwrap();
    pump(app);
    assert_eq!(app.editor().selection(), 0..0);
    app.remove_document(empty).unwrap();
    pump(app);
    app.command(RESULTS_NEXT).unwrap();
    assert!(app.note.contains("document was closed"));
    app.command(RESULTS_CLEAR).unwrap();
    assert!(app.results.links.is_empty());
    app.command(RESULTS_CLOSE).unwrap();
    app.command(SEARCH_CLOSE).unwrap();
    app.remove_document(second).unwrap();
    pump(app);
    assert_eq!(app.index(), first);
    text(app, "");
    app.search.query.set_text("");
    app.search.mode.set_active(Some(0));
    assert!(app.snapshot().unwrap().show_symbols.all_characters());
}

#[test]
fn gtk_workflows_preserve_editing_features_and_recovery() {
    gtk::init().expect("Run GUI tests in a desktop session or with xvfb-run");
    let directory = std::env::temp_dir().join(format!(
        "rstpd-gtk-workflows-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let _cleanup = DirectoryCleanup(directory.clone());
    let state = directory.join("state");
    let mut app = App::new(&state).unwrap();
    assert!(
        App::new(&state).is_err(),
        "A second instance must not share recovery"
    );
    app.new_document().unwrap();
    app.window.show_all();
    app.layout();
    pump(&mut app);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.documents[0].snapshot.eol, Eol::Lf);
    assert_eq!(
        gtk::Settings::default()
            .unwrap()
            .property::<String>("gtk-theme-name"),
        "Adwaita"
    );
    assert!(!app.search.container.is_visible());
    assert!(!app.tree_scroll.is_visible());
    assert!(!app.map_box.is_visible());
    assert!(!app.editors[1].widget().is_visible());
    let page = app.tabs.nth_page(Some(0)).unwrap();
    let tab = app
        .tabs
        .tab_label(&page)
        .unwrap()
        .downcast::<gtk::Box>()
        .unwrap();
    let label = tab.children()[0].clone().downcast::<gtk::Label>().unwrap();
    wait_for(&mut app, |_| label.allocated_width() > 100);
    assert!(
        !label
            .layout()
            .expect("A mapped tab must have a text layout")
            .is_ellipsized(),
        "An ordinary tab title must not collapse to an ellipsis"
    );
    let icon_theme = gtk::IconTheme::default().unwrap();
    for button in TOOLS {
        assert!(
            icon_theme.has_icon(button.icon),
            "Missing standard icon: {}",
            button.icon
        );
    }
    let file = app.menu.children()[0]
        .clone()
        .downcast::<gtk::MenuItem>()
        .unwrap();
    let menu = file.submenu().unwrap().downcast::<gtk::Menu>().unwrap();
    menu.children()[0].emit_by_name::<()>("activate", &[]);
    pump(&mut app);
    assert_eq!(
        app.documents.len(),
        2,
        "Menu activation must reach the command queue"
    );
    app.tabs.set_current_page(Some(0));
    pump(&mut app);
    assert_eq!(
        app.index(),
        0,
        "Native tab changes must select the corresponding document"
    );
    app.remove_document(1).unwrap();
    pump(&mut app);
    app.editor()
        .replace(0..0, "changed before switching")
        .unwrap();
    app.new_document().unwrap();
    pump(&mut app);
    assert!(
        app.documents[0].snapshot.dirty,
        "Deferred notifications must refer to the original document"
    );
    assert!(
        !app.documents[1].snapshot.dirty,
        "Switching panes must not dirty another document"
    );
    app.remove_document(0).unwrap();
    pump(&mut app);
    exercise_editor_font_selection(&mut app);
    exercise_symbols_results_and_counts(&mut app);

    text(&mut app, "one\ntwo\n");
    app.command(SPLIT).unwrap();
    pump(&mut app);
    assert!(app.editors[1].widget().is_visible());
    app.editors[0].send(SCI_SETSELECTION, 3, 0);
    app.editors[0].send(SCI_ADDSELECTION, 7, 4);
    app.command(UPPER).unwrap();
    pump(&mut app);
    assert_eq!(app.editors[1].text().unwrap(), "ONE\nTWO\n");
    app.command(UNDO).unwrap();
    pump(&mut app);
    assert_eq!(app.editor().text().unwrap(), "one\ntwo\n");
    app.event(Event::OtherPane).unwrap();
    assert_eq!(app.focused, 1);
    app.command(SPLIT).unwrap();
    pump(&mut app);
    assert_eq!(app.focused, 0);

    for (command, operation) in [
        (LOWER, CaseOp::Lower),
        (UPPER, CaseOp::Upper),
        (TITLE_CASE, CaseOp::Title),
        (SENTENCE_CASE, CaseOp::Sentence),
        (INVERT_CASE, CaseOp::Invert),
    ] {
        text(&mut app, "aBc DeF. next WORD");
        app.editor().select(0..app.editor().length());
        app.command(command).unwrap();
        pump(&mut app);
        assert_eq!(
            app.editor().text().unwrap(),
            core::change_case("aBc DeF. next WORD", operation)
        );
        app.command(UNDO).unwrap();
        pump(&mut app);
        assert_eq!(app.editor().text().unwrap(), "aBc DeF. next WORD");
    }
    for (command, operation, input) in [
        (SORT, LineOp::Sort, "b\nA\na\n"),
        (SORT_DESC, LineOp::SortDescending, "b\nA\na\n"),
        (SORT_IGNORE_CASE, LineOp::SortIgnoreCase, "b\nA\na\n"),
        (
            SORT_DESC_IGNORE_CASE,
            LineOp::SortDescendingIgnoreCase,
            "b\nA\na\n",
        ),
        (SORT_NATURAL, LineOp::SortNatural, "file10\nfile2\nfile1\n"),
        (
            SORT_NUMERIC,
            LineOp::SortNumeric,
            "100000000000000000000\n3\n21\n",
        ),
        (
            SORT_NUMERIC_DESC,
            LineOp::SortNumericDescending,
            "2.1\n2.01\n-1\n",
        ),
        (
            SORT_NUMERIC_COMMA,
            LineOp::SortNumericComma,
            "2,1\n2,01\n-1\n",
        ),
        (REVERSE_LINES, LineOp::Reverse, "a\nb\nc\n"),
        (UNIQUE, LineOp::Unique, "a\nb\na\n"),
        (UNIQUE_ADJACENT, LineOp::UniqueAdjacent, "a\na\nb\na\n"),
        (TRIM, LineOp::Trim, " a  \nb \n"),
        (TRIM_START, LineOp::TrimStart, " a  \n b\n"),
        (TRIM_BOTH, LineOp::TrimBoth, " a  \n b \n"),
        (REMOVE_EMPTY, LineOp::RemoveEmpty, "a\n \n\nb\n"),
        (REMOVE_EMPTY_ONLY, LineOp::RemoveEmptyOnly, "a\n \n\nb\n"),
        (JOIN_LINES, LineOp::Join, "a\nb\nc\n"),
    ] {
        text(&mut app, input);
        app.command(command).unwrap();
        pump(&mut app);
        assert_eq!(
            app.editor().text().unwrap(),
            core::lines(input, operation, Eol::Lf).unwrap(),
            "command {command}"
        );
    }
    text(&mut app, "3\ninvalid\n1\n");
    assert!(app.command(SORT_NUMERIC).is_err());
    assert_eq!(app.editor().text().unwrap(), "3\ninvalid\n1\n");

    text(&mut app, "prefix:echo echo end\nprefix:stay stay!");
    app.command(FIND).unwrap();
    assert!(app.search.container.is_visible());
    app.search.mode.set_active(Some(2));
    app.search.query.set_text(r"(?<=prefix:)(\w+)\s+\1(?!x)");
    app.search.replace.set_text("<$1>");
    app.command(FIND_NEXT).unwrap();
    assert_eq!(
        app.editor().range(app.editor().selection()).unwrap(),
        "echo echo"
    );
    app.command(REPLACE_ALL).unwrap();
    pump(&mut app);
    assert_eq!(
        app.editor().text().unwrap(),
        "prefix:<echo> end\nprefix:<stay>!"
    );
    app.command(UNDO).unwrap();
    pump(&mut app);
    app.search.mode.set_active(Some(1));
    app.search.query.set_text(r"\n");
    app.command(FIND_NEXT).unwrap();
    assert_eq!(app.editor().range(app.editor().selection()).unwrap(), "\n");
    app.command(SEARCH_CLOSE).unwrap();
    assert!(!app.search.container.is_visible());
    text(&mut app, "ab");
    app.search.mode.set_active(Some(2));
    app.search.query.set_text("(?=.)");
    app.command(FIND_NEXT).unwrap();
    let first = app.editor().position();
    app.command(FIND_NEXT).unwrap();
    assert!(
        app.editor().position() > first,
        "Zero-width searches must advance"
    );

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    app.import_language(&fixtures.join("custom-language.xml"))
        .unwrap();
    let custom = app
        .languages
        .iter()
        .position(|language| language.name == "Example data language")
        .unwrap();
    app.command(LANGUAGE_BASE + custom).unwrap();
    text(
        &mut app,
        include_str!("../../tests/fixtures/sample.rstlang"),
    );
    wait_for(&mut app, |app| {
        app.documents[app.index()].styled_revision == Some(app.documents[app.index()].revision)
    });
    assert_eq!(
        app.editor().send(
            SCI_GETSTYLEAT,
            app.editor().text().unwrap().find("say").unwrap(),
            0
        ),
        4
    );
    app.import_api(&fixtures.join("completion-api.xml"))
        .unwrap();
    assert!(
        app.completion_api
            .iter()
            .any(|api| api.name == "lookup_record")
    );
    app.command(REMOVE_LANGUAGE).unwrap();
    assert_eq!(
        app.languages[app.documents[app.index()].language].name,
        "Plain text"
    );
    app.import_language(&fixtures.join("custom-language.xml"))
        .unwrap();
    let rust = languages::detect(Path::new("test.rs"), &app.languages);
    app.command(LANGUAGE_BASE + rust).unwrap();
    app.import_api(&fixtures.join("completion-api.xml"))
        .unwrap();
    text(&mut app, "let text = String::from(\"hello\");\ntext.tr");
    app.editor().send(SCI_GOTOPOS, app.editor().length(), 0);
    app.command(COMPLETE).unwrap();
    assert_ne!(app.editor().send(SCI_AUTOCACTIVE, 0, 0), 0);
    app.editor().send(SCI_AUTOCCANCEL, 0, 0);
    text(&mut app, include_str!("../../tests/fixtures/functions.rs"));
    app.editor().send(SCI_GOTOPOS, app.editor().length(), 0);
    app.command(PARAMETER_HINT).unwrap();
    assert_ne!(app.editor().send(SCI_CALLTIPACTIVE, 0, 0), 0);
    app.editor().send(SCI_CALLTIPCANCEL, 0, 0);

    let markdown = languages::detect(Path::new("test.md"), &app.languages);
    app.command(LANGUAGE_BASE + markdown).unwrap();
    text(
        &mut app,
        include_str!("../../tests/fixtures/highlighting.md"),
    );
    wait_for(&mut app, |app| {
        app.documents[app.index()].styled_revision == Some(app.documents[app.index()].revision)
    });
    for command in [THEME_LIGHT, THEME_DARK, THEME_SYSTEM] {
        app.command(command).unwrap();
        pump(&mut app);
        let editor = app.editor();
        let background = app.palette.background;
        let brightness =
            (background & 255) + ((background >> 8) & 255) + ((background >> 16) & 255);
        if command == THEME_LIGHT {
            assert!(
                brightness > 3 * 128,
                "The Adwaita light editor must have a light background"
            );
        } else if command == THEME_DARK {
            assert!(
                brightness < 3 * 128,
                "The Adwaita dark editor must have a dark background"
            );
        }
        assert_eq!(editor.send(SCI_STYLEGETBACK, 32, 0), background as isize);
        let text = editor.text().unwrap();
        assert!(
            contrast(app.palette.text, background) >= 4.5,
            "Body text must remain readable"
        );
        let link_style =
            editor.send(SCI_GETSTYLEAT, text.find("the documentation").unwrap(), 0) as usize;
        assert!(
            contrast(
                editor.send(SCI_STYLEGETFORE, link_style, 0) as u32,
                background
            ) >= 3.0,
            "Use the theme's link foreground, not a selection-background color, for link text"
        );
        let style = editor.send(SCI_GETSTYLEAT, text.find("bold words").unwrap(), 0) as usize;
        assert_ne!(editor.send(SCI_STYLEGETBOLD, style, 0), 0);
        assert_ne!(
            editor.send(
                SCI_INDICATORVALUEAT,
                crate::markdown::STRIKE_INDICATOR,
                text.find("removed words").unwrap() as isize
            ),
            0
        );
    }
    text(
        &mut app,
        &(0..1000).map(|i| format!("Line {i}\n")).collect::<String>(),
    );
    app.command(MAP).unwrap();
    pump(&mut app);
    assert!(app.map_box.is_visible());
    assert_eq!(app.map.send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 200);
    assert_eq!(app.editor().send(SCI_GETREADONLY, 0, 0), 0);
    app.event(Event::MapScroll(20)).unwrap();
    assert!(app.map.send(SCI_GETFIRSTVISIBLELINE, 0, 0) > 0);
    app.event(Event::Map(20)).unwrap();
    assert!(app.editor().position() > 0);
    app.editor().replace(0..0, "Editable\n").unwrap();
    app.command(MAP).unwrap();
    app.command(WRAP).unwrap();
    assert_eq!(app.editor().send(SCI_GETWRAPMODE, 0, 0), 1);
    app.command(WRAP).unwrap();

    text(&mut app, "{one:0xFF, list:[1,2,], /* comment */ label:'a'}");
    app.command(JSON_FORMAT).unwrap();
    pump(&mut app);
    assert!(app.editor().text().unwrap().contains("0xFF"));
    assert!(app.editor().text().unwrap().contains("/* comment */"));
    app.command(JSON_COMPACT).unwrap();
    pump(&mut app);
    assert!(app.editor().text().unwrap().contains("0xFF"));
    text(&mut app, "{\"a\":[1,2]}");
    app.command(JSON_TREE).unwrap();
    wait_for(&mut app, |app| {
        app.json_document == Some(app.documents[app.index()].snapshot.id)
    });
    let index = app
        .json_nodes
        .iter()
        .position(|node| node.pointer == "/a/1")
        .unwrap();
    app.event(Event::Tree(index)).unwrap();
    assert_eq!(app.editor().range(app.editor().selection()).unwrap(), "2");
    let branch = app
        .json_nodes
        .iter()
        .position(|node| node.pointer == "/a")
        .unwrap();
    let branch_path = app.tree_store.path(&app.json_handles[branch]).unwrap();
    app.tree.expand_row(&branch_path, false);
    app.editor().replace(0..1, "").unwrap();
    pump(&mut app);
    assert!(
        !app.tree.is_sensitive(),
        "Stale JSON spans must never remain navigable"
    );
    wait_for(&mut app, |app| app.note.starts_with("JSON paused:"));
    app.editor().replace(0..0, "{").unwrap();
    wait_for(&mut app, |app| {
        app.json_document == Some(app.documents[app.index()].snapshot.id)
    });
    let branch = app
        .json_nodes
        .iter()
        .position(|node| node.pointer == "/a")
        .unwrap();
    assert!(
        app.tree
            .row_expanded(&app.tree_store.path(&app.json_handles[branch]).unwrap())
    );
    app.command(JSON_TREE).unwrap();

    app.new_document().unwrap();
    let left = app.index();
    text(&mut app, "one\nold\nthree\n");
    app.new_document().unwrap();
    text(&mut app, "one\nchanged\nthree\n");
    app.switch(left).unwrap();
    app.command(COMPARE).unwrap();
    wait_for(&mut app, |app| {
        app.compare_rx.is_none() && app.compare_due.is_none() && !app.differences.is_empty()
    });
    assert_ne!(app.editors[0].send(SCI_MARKERGET, 1, 0), 0);
    app.command(DIFF_NEXT).unwrap();
    app.command(DIFF_PREVIOUS).unwrap();
    app.editors[0].replace(4..7, "changed").unwrap();
    app.editors[0].send(SCI_GOTOPOS, 1, 0);
    wait_for(&mut app, |app| {
        app.compare_rx.is_none() && app.compare_due.is_none() && app.differences.is_empty()
    });
    assert_eq!(
        app.editors[0].position(),
        1,
        "Background compare must not move the caret"
    );
    app.command(COMPARE_CLEAR).unwrap();
    app.command(SPLIT).unwrap();
    pump(&mut app);

    let encoded = directory.join("encoded.txt");
    text(&mut app, "caf\u{e9}\ntext\n");
    app.command(EOL_CRLF).unwrap();
    let encoding = core::encoding_options()
        .iter()
        .position(|e| *e == Encoding::Utf16Be)
        .unwrap();
    app.command(ENCODING_BASE + encoding).unwrap();
    pump(&mut app);
    app.save_to(&encoded, true).unwrap();
    pump(&mut app);
    let bytes = fs::read(&encoded).unwrap();
    assert!(bytes.starts_with(b"\xfe\xff"));
    assert_eq!(
        core::decode(&bytes, None).unwrap().0,
        "caf\u{e9}\r\ntext\r\n"
    );
    assert!(!app.documents[app.index()].snapshot.dirty);
    app.new_document().unwrap();
    assert!(
        app.save_to(&encoded, true).is_err(),
        "Two tabs cannot overwrite the same file"
    );
    let dropped = directory.join("name with spaces.rs");
    fs::write(&dropped, "fn main() {}\n").unwrap();
    app.event(Event::Uris(gio::File::for_path(&dropped).uri().into()))
        .unwrap();
    pump(&mut app);
    assert_eq!(
        app.documents[app.index()].snapshot.path.as_ref(),
        Some(&dropped)
    );
    assert!(
        app.event(Event::Uris("https://example.invalid/remote.txt".into()))
            .is_err()
    );

    app.new_document().unwrap();
    app.editor().replace(0..0, "recover me \u{1f680}").unwrap();
    app.editor().send(SCI_GOTOPOS, 3, 0);
    pump(&mut app);
    let recovered_count = app.documents.len();
    app.close().unwrap();
    drop(app);
    EVENTS.with(|events| events.borrow_mut().clear());
    let mut app = App::new(&state).unwrap();
    app.window.show_all();
    app.layout();
    pump(&mut app);
    assert_eq!(app.documents.len(), recovered_count);
    assert_eq!(app.editor().text().unwrap(), "recover me \u{1f680}");
    assert_eq!(app.editor().position(), 3);
    assert!(app.show_symbols.all_characters());
    assert_eq!(app.editor().send(SCI_GETVIEWWS, 0, 0), 1);
    assert_eq!(app.editor().send(SCI_GETVIEWEOL, 0, 0), 1);
    assert!(
        !app.results.container.is_visible(),
        "Search results are transient"
    );
    assert!(app.status.text().contains("12 characters"));
    assert!(app.documents[app.index()].snapshot.dirty);
    assert_eq!(app.editor_font, EditorFont::new("Sans", 1250).unwrap());
    assert_eq!(style_font(&app.editor(), 32), "Sans");
    assert_eq!(app.editor().send(SCI_STYLEGETSIZEFRACTIONAL, 32, 0), 1250);
    assert!(
        app.languages
            .iter()
            .any(|language| language.name == "Example data language")
    );
    assert!(
        app.completion_api
            .iter()
            .any(|api| api.name == "lookup_record")
    );
    let clean = app
        .documents
        .iter()
        .position(|doc| doc.snapshot.path.as_ref() == Some(&encoded))
        .unwrap();
    app.switch(clean).unwrap();
    app.close_document().unwrap();
    assert_eq!(app.documents.len(), recovered_count - 1);
    app.close().unwrap();
    drop(app);
    EVENTS.with(|events| events.borrow_mut().clear());
}
