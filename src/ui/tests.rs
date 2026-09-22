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
            app.search.query.clone().upcast::<gtk::Widget>(),
            app.search.replace.clone().upcast::<gtk::Widget>(),
            app.search.mode.clone().upcast::<gtk::Widget>(),
            app.search.case.clone().upcast::<gtk::Widget>(),
            app.search.word.clone().upcast::<gtk::Widget>(),
        ]
        .into_iter()
        .chain(
            app.search
                .buttons
                .iter()
                .map(|(_, button)| button.clone().upcast::<gtk::Widget>()),
        )
        .filter(|control| control.is_drawable())
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

fn with_message_response(
    app: &mut App,
    response: gtk::ResponseType,
    action: impl FnOnce(&mut App),
) {
    let observed = Rc::new(Cell::new(false));
    let capture = observed.clone();
    let timer = glib::timeout_add_local(Duration::from_millis(10), move || {
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|window| window.downcast::<gtk::MessageDialog>().ok());
        if let Some(dialog) = dialog {
            capture.set(true);
            dialog.response(response);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    action(app);
    if !observed.get() {
        timer.remove();
    }
    assert!(observed.get(), "Expected a native confirmation dialog");
}

fn document_index(app: &App, id: u64) -> usize {
    app.documents
        .iter()
        .position(|doc| doc.snapshot.id == id)
        .unwrap()
}

fn press(widget: &gtk::Widget, kind: gdk::EventType, button: u32) -> bool {
    press_at(widget, kind, button, (0.0, 0.0))
}

fn press_at(widget: &gtk::Widget, kind: gdk::EventType, button: u32, position: (f64, f64)) -> bool {
    let mut event = gdk::Event::new(kind);
    let native: &mut gdk::ffi::GdkEventButton =
        event.downcast_mut::<gdk::EventButton>().unwrap().as_mut();
    native.button = button;
    native.x = position.0;
    native.y = position.1;
    let value: glib::Value = event.into();
    widget.emit_by_name("button-press-event", &[&value])
}

fn exercise_tab_order(app: &mut App) {
    let original = app.documents[app.index()].snapshot.id;
    app.new_document().unwrap();
    let edited = app.documents[app.index()].snapshot.id;
    app.editor().replace(0..0, "retained undo").unwrap();
    app.new_document().unwrap();
    let right = app.documents[app.index()].snapshot.id;
    pump(app);
    app.command(SPLIT).unwrap();
    app.focused = 1;
    app.switch(document_index(app, original)).unwrap();
    pump(app);
    let panes = app.pane_documents();
    app.event(Event::PinTab(edited)).unwrap();
    app.event(Event::PinTab(original)).unwrap();
    assert_eq!(app.documents[0].snapshot.id, edited);
    assert_eq!(app.pane_documents(), panes);
    assert_eq!(app.documents[app.index()].snapshot.id, original);
    let page = app
        .tabs
        .nth_page(Some(document_index(app, right) as u32))
        .unwrap();
    assert!(app.tabs.child_is_reorderable(&page));
    app.tabs.reorder_child(&page, Some(0));
    pump(app);
    assert_eq!(
        app.documents.last().unwrap().snapshot.id,
        right,
        "Unpinned tabs cannot cross the pinned partition"
    );
    assert_eq!(app.pane_documents(), panes);
    app.event(Event::MoveTab(original, 0)).unwrap();
    app.event(Event::PinTab(edited)).unwrap();
    app.event(Event::StepTab(original, false)).unwrap();
    assert_eq!(app.documents[0].snapshot.id, original);
    let edited_index = document_index(app, edited);
    app.scratch.attach(&app.documents[edited_index].handle);
    assert_eq!(app.scratch.text().unwrap(), "retained undo");
    assert_ne!(app.scratch.send(SCI_CANUNDO, 0, 0), 0);
    assert!(app.documents[edited_index].snapshot.dirty);
    assert!(app.snapshot().unwrap().documents[0].pinned);

    let strip = app.tabs.parent().unwrap().downcast::<gtk::Box>().unwrap();
    let empty_strip = strip.children()[1].clone();
    assert!(empty_strip.tooltip_text().unwrap().contains("Double-click"));
    app.window.resize(1280, 840);
    wait_for(app, |app| {
        let last = app.tabs.nth_page(Some(app.tabs.n_pages() - 1)).unwrap();
        app.tabs.allocated_width() == app.tab_width.get()
            && app.tabs.tab_label(&last).unwrap().allocated_width() > 100
            && app.tabs.tab_label(&last).unwrap().is_drawable()
            && empty_strip.allocated_width() > 48
    });
    let (tab_x, _) = app.tabs.translate_coordinates(&strip, 0, 0).unwrap();
    let (blank_x, blank_y) = empty_strip.translate_coordinates(&strip, 0, 0).unwrap();
    assert_eq!(
        blank_x,
        tab_x + app.tabs.allocated_width(),
        "The expandable blank strip must start immediately after the full native tab header"
    );
    let last = app.tabs.nth_page(Some(app.tabs.n_pages() - 1)).unwrap();
    let last_label = app.tabs.tab_label(&last).unwrap();
    let (last_x, _) = last_label.translate_coordinates(&strip, 0, 0).unwrap();
    assert!(blank_x >= last_x + last_label.allocated_width());
    let boundary = (blank_x + 1, blank_y + empty_strip.allocated_height() / 2);
    let (x, y) = strip
        .translate_coordinates(&empty_strip, boundary.0, boundary.1)
        .unwrap();
    assert_eq!(
        x, 1,
        "Exercise the first blank pixel after the tab boundary, not the far-right action area"
    );
    let position = (f64::from(x), f64::from(y));
    let count = app.documents.len();
    assert!(!press_at(
        &empty_strip,
        gdk::EventType::ButtonPress,
        1,
        position
    ));
    assert!(!press_at(
        &empty_strip,
        gdk::EventType::DoubleButtonPress,
        3,
        position
    ));
    let page = app.tabs.nth_page(Some(0)).unwrap();
    let label = app.tabs.tab_label(&page).unwrap();
    press(&label, gdk::EventType::DoubleButtonPress, 1);
    pump(app);
    assert_eq!(app.documents.len(), count);
    assert!(press_at(
        &empty_strip,
        gdk::EventType::DoubleButtonPress,
        1,
        position
    ));
    pump(app);
    assert_eq!(app.documents.len(), count + 1);
    assert!(!app.documents.last().unwrap().snapshot.pinned);
    app.command(SPLIT).unwrap();
    while let Some(index) = app
        .documents
        .iter()
        .position(|doc| doc.snapshot.id != original)
    {
        app.remove_document(index).unwrap();
    }
    app.switch(document_index(app, original)).unwrap();
    pump(app);
}

fn exercise_external_reload(app: &mut App, directory: &Path) {
    let original = app.documents[app.index()].snapshot.id;
    let path = directory.join("monitor.txt");
    let initial: String = (0..160).map(|line| format!("line {line:03}\n")).collect();
    fs::write(&path, &initial).unwrap();
    app.open_path(&path, None).unwrap();
    let id = app.documents[app.index()].snapshot.id;
    app.command(SPLIT).unwrap();
    app.editors[0].select(90..99);
    app.editors[0].send(SCI_ADDSELECTION, 117, 108);
    app.editors[1].select(180..189);
    app.editors[0].send(SCI_SETFIRSTVISIBLELINE, 8, 0);
    app.editors[1].send(SCI_SETFIRSTVISIBLELINE, 16, 0);
    pump(app);
    let views: Vec<_> = app
        .editors
        .iter()
        .map(|editor| {
            (
                editor.selection(),
                editor.position(),
                editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0),
            )
        })
        .collect();
    let updated = initial.replace("line", "text");
    fs::write(&path, &updated).unwrap();
    app.last_monitor = Instant::now() - Duration::from_secs(2);
    wait_for(app, |app| {
        app.documents[document_index(app, id)].snapshot.disk_hash
            == Some(session::fingerprint(updated.as_bytes()))
    });
    pump(app);
    assert_eq!(app.editors[0].text().unwrap(), updated);
    assert!(!app.document_dirty(document_index(app, id)));
    assert_eq!(app.editors[0].send(SCI_GETSELECTIONS, 0, 0), 2);
    assert_eq!(app.editors[0].send(SCI_GETSELECTIONNANCHOR, 0, 0), 90);
    assert_eq!(app.editors[0].send(SCI_GETSELECTIONNCARET, 0, 0), 99);
    for (editor, (selection, caret, scroll)) in app.editors.iter().zip(views) {
        assert_eq!(editor.selection(), selection);
        assert_eq!(editor.position(), caret);
        assert_eq!(editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0), scroll);
    }
    wait_for(app, |app| !app.monitor_busy);
    let paused = format!("{updated}changed while monitoring was paused\n");
    fs::write(&path, &paused).unwrap();
    app.last_monitor = Instant::now() - Duration::from_secs(2);
    app.poll_monitor();
    assert!(app.monitor_busy);
    app.command(MONITOR_FILES).unwrap();
    assert!(!app.monitor_files && app.monitor_busy);
    wait_for(app, |app| !app.monitor_busy);
    assert_eq!(
        app.editor().text().unwrap(),
        updated,
        "Disabled monitoring must discard the outstanding result"
    );
    assert_eq!(
        app.documents[document_index(app, id)].snapshot.disk_hash,
        Some(session::fingerprint(updated.as_bytes()))
    );
    app.command(MONITOR_FILES).unwrap();
    wait_for(app, |app| {
        app.documents[document_index(app, id)].snapshot.disk_hash
            == Some(session::fingerprint(paused.as_bytes()))
    });
    assert_eq!(
        app.editor().text().unwrap(),
        paused,
        "Re-enabling must rescan an unchanged external version whose earlier result was discarded"
    );
    let baseline = app.documents[document_index(app, id)].snapshot.disk_hash;
    let revision = app.documents[document_index(app, id)].revision;
    app.editor().replace(0..0, "unsaved ").unwrap();
    assert_eq!(
        app.documents[document_index(app, id)].revision,
        revision,
        "Dirty notification is deliberately still queued"
    );
    let external = format!("external\n{updated}");
    fs::write(&path, &external).unwrap();
    with_message_response(app, gtk::ResponseType::Cancel, |app| {
        app.apply_external_change(monitor::Change {
            id,
            path: fs::canonicalize(&path).unwrap(),
            baseline_hash: baseline,
            result: Ok(monitor::Snapshot {
                hash: session::fingerprint(external.as_bytes()),
                text: external.clone(),
                encoding: Encoding::Utf8,
                eol: Eol::Lf,
            }),
        })
        .unwrap();
    });
    assert!(app.editor().text().unwrap().starts_with("unsaved "));
    assert_eq!(
        app.documents[document_index(app, id)].snapshot.disk_hash,
        baseline
    );
    with_message_response(app, gtk::ResponseType::Cancel, |app| {
        assert!(
            !app.save_document(false).unwrap(),
            "Save must still warn after declining reload"
        );
    });
    // Disable the asynchronous scan while checking a manually delivered worker reply.
    app.command(MONITOR_FILES).unwrap();
    pump(app);
    app.last_autosave = Instant::now() - Duration::from_secs(4);
    app.touch();
    wait_for(app, |app| {
        !app.recovery_busy && app.recovered_revision == app.revision
    });
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        external,
        "Recovery must not overwrite the original file"
    );
    with_message_response(app, gtk::ResponseType::Yes, |app| {
        app.command(MONITOR_FILES).unwrap();
        wait_for(app, |app| {
            app.documents[document_index(app, id)].snapshot.disk_hash
                == Some(session::fingerprint(external.as_bytes()))
        });
    });
    pump(app);
    assert_eq!(app.editor().text().unwrap(), external);
    assert!(!app.document_dirty(document_index(app, id)));
    app.apply_external_change(monitor::Change {
        id,
        path: fs::canonicalize(&path).unwrap(),
        baseline_hash: baseline,
        result: Ok(monitor::Snapshot {
            hash: 0,
            text: "stale worker reply".into(),
            encoding: Encoding::Utf8,
            eol: Eol::Lf,
        }),
    })
    .unwrap();
    assert_eq!(app.editor().text().unwrap(), external);
    app.command(SPLIT).unwrap();
    app.remove_document(document_index(app, id)).unwrap();
    app.switch(document_index(app, original)).unwrap();
    assert!(app.monitor_files);
    pump(app);
}

fn exercise_folder_search(app: &mut App, directory: &Path) {
    let original = app.documents[app.index()].snapshot.id;
    let folder = directory.join("folder-search");
    fs::create_dir(&folder).unwrap();
    fs::create_dir(folder.join("nested")).unwrap();
    for (path, contents) in [
        (folder.join("a.txt"), "needle current"),
        (folder.join("skip.txt"), "needle excluded"),
        (folder.join("other.rs"), "needle source"),
        (folder.join(".hidden.txt"), "needle hidden"),
        (folder.join("nested").join("deep.txt"), "needle deep"),
    ] {
        fs::write(path, contents).unwrap();
    }
    app.command(MONITOR_FILES).unwrap();
    app.command(FIND_FILES).unwrap();
    app.search.query.set_text("needle");
    app.search.mode.set_active(Some(0));
    app.search.directory.set_text(&folder.to_string_lossy());
    app.search.filters.set_text("*.txt;!skip*");
    app.search.recursive.set_active(false);
    app.search.hidden.set_active(false);
    app.command(FIND_FILES_RUN).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.results.data.as_ref().unwrap().count(), 1);
    assert!(
        !app.note.contains("some documents changed"),
        "Disk result IDs are not open-tab IDs"
    );
    app.command(RESULTS_NEXT).unwrap();
    pump(app);
    assert_eq!(
        app.editor().range(app.editor().selection()).unwrap(),
        "needle"
    );
    let opened = app.documents[app.index()].snapshot.id;
    app.remove_document(document_index(app, opened)).unwrap();
    app.command(RESULTS_NEXT).unwrap();
    pump(app);
    let reopened = app.documents[app.index()].snapshot.id;
    assert_ne!(opened, reopened, "Disk results may reopen a closed file");
    app.editor().replace(0..0, "unsaved ").unwrap();
    let caret = app.editor().position();
    app.command(RESULTS_NEXT).unwrap();
    assert!(app.note.contains("changed since the search"));
    assert_eq!(app.editor().position(), caret);
    app.command(UNDO).unwrap();
    pump(app);
    fs::write(folder.join("a.txt"), "different disk data").unwrap();
    app.command(RESULTS_NEXT).unwrap();
    assert!(app.note.contains("changed or is unavailable"));
    assert_eq!(app.editor().text().unwrap(), "needle current");
    app.remove_document(document_index(app, reopened)).unwrap();
    fs::write(folder.join("a.txt"), "needle current").unwrap();
    app.search.recursive.set_active(true);
    app.command(FIND_FILES_RUN).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.results.data.as_ref().unwrap().count(), 2);
    app.search.hidden.set_active(true);
    app.command(FIND_FILES_RUN).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert_eq!(app.results.data.as_ref().unwrap().count(), 3);
    fs::write(folder.join("binary.txt"), b"needle\0binary").unwrap();
    app.command(FIND_FILES_RUN).unwrap();
    wait_for(app, |app| app.results.job.is_none());
    assert!(app.results.data.as_ref().unwrap().skipped > 0);
    assert!(!app.results.data.as_ref().unwrap().warnings.is_empty());
    app.command(RESULTS_CLEAR).unwrap();
    app.command(RESULTS_CLOSE).unwrap();
    app.command(SEARCH_CLOSE).unwrap();
    app.search.folder_visible = false;
    app.search.query.set_text("");
    app.switch(document_index(app, original)).unwrap();
    app.command(MONITOR_FILES).unwrap();
    pump(app);
}

fn wait_for_comparison(app: &mut App) {
    wait_for(app, |app| {
        app.compare_rx.is_none() && app.compare_due.is_none()
    });
    assert!(!app.note.starts_with("Compare paused:"), "{}", app.note);
}

fn respond_to_compare_options(app: &mut App, options: CompareOptions, response: gtk::ResponseType) {
    let observed = Rc::new(Cell::new(false));
    let capture = observed.clone();
    let timer = glib::timeout_add_local(Duration::from_millis(10), move || {
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .filter(|window| window.title().as_deref() == Some("Compare options"))
            .find_map(|window| window.downcast::<gtk::Dialog>().ok());
        let Some(dialog) = dialog else {
            return glib::ControlFlow::Continue;
        };
        let children = dialog.content_area().children();
        let checks: Vec<_> = children
            .iter()
            .filter_map(|child| child.clone().downcast::<gtk::CheckButton>().ok())
            .collect();
        assert_eq!(checks.len(), 5);
        for (check, value) in checks.iter().zip([
            options.ignore_whitespace,
            options.ignore_case,
            options.ignore_empty_lines,
            options.detect_moves,
            options.align,
        ]) {
            check.set_active(value);
        }
        let regex = children
            .into_iter()
            .find_map(|child| child.downcast::<gtk::Entry>().ok())
            .unwrap();
        regex.set_text(&options.ignore_regex);
        capture.set(true);
        dialog.response(response);
        glib::ControlFlow::Break
    });
    app.command(COMPARE_SETTINGS).unwrap();
    if !observed.get() {
        timer.remove();
    }
    assert!(observed.get());
}

fn exercise_compare_modes(app: &mut App, directory: &Path) {
    let original = app.documents[app.index()].snapshot.id;
    app.new_document().unwrap();
    let left = app.documents[app.index()].snapshot.id;
    text(app, "prefix left\nsame\nold value\nend\n");
    app.command(SPLIT).unwrap();
    app.editors[0].select(13..17);
    app.editors[1].select(13..17);
    assert!(
        app.command(COMPARE_SELECTION)
            .unwrap_err()
            .contains("two different documents")
    );
    assert!(!app.comparing && app.compare_pair.is_none());
    app.command(SPLIT).unwrap();
    app.new_document().unwrap();
    let right = app.documents[app.index()].snapshot.id;
    text(app, "prefix right\nextra\nsame\nnew value\nend\n");
    app.begin_compare(document_index(app, left), document_index(app, right), None)
        .unwrap();
    wait_for_comparison(app);
    let texts = [
        app.editors[0].text().unwrap(),
        app.editors[1].text().unwrap(),
    ];
    let undo = [
        app.editors[0].send(SCI_CANUNDO, 0, 0),
        app.editors[1].send(SCI_CANUNDO, 0, 0),
    ];
    assert_ne!(app.editors[0].send(SCI_ANNOTATIONGETLINES, 0, 0), 0);
    app.command(WRAP).unwrap();
    assert_eq!(
        app.editors[0].send(SCI_GETWRAPMODE, 0, 0),
        0,
        "Alignment temporarily disables wrap"
    );
    app.editors[0].select(13..17);
    app.editors[1].select(21..23);
    app.command(COMPARE_SELECTION).unwrap();
    wait_for_comparison(app);
    assert!(
        app.differences.is_empty(),
        "Only the selected complete 'same' lines are compared"
    );
    assert_eq!(app.compare_selection, Some([1..2, 2..3]));
    assert_eq!(app.editors[0].text().unwrap(), texts[0]);
    assert_eq!(app.editors[1].text().unwrap(), texts[1]);
    for (pane, editor) in app.editors.iter().enumerate() {
        assert_eq!(editor.send(SCI_CANUNDO, 0, 0), undo[pane]);
    }
    let old_generation = app.compare_generation;
    let options = CompareOptions {
        ignore_whitespace: true,
        ignore_case: true,
        ignore_empty_lines: true,
        ignore_regex: "[0-9]+".into(),
        ..CompareOptions::default()
    };
    respond_to_compare_options(app, options.clone(), gtk::ResponseType::Cancel);
    assert!(!app.compare_options.ignore_case);
    assert_eq!(app.compare_generation, old_generation);
    respond_to_compare_options(app, options, gtk::ResponseType::Ok);
    wait_for_comparison(app);
    app.apply_compare(CompareResult {
        left,
        right,
        left_rev: app.documents[document_index(app, left)].revision,
        right_rev: app.documents[document_index(app, right)].revision,
        generation: old_generation,
        options: format!("{:?}", app.compare_options),
        result: Err("obsolete options reply".into()),
    })
    .unwrap();
    assert!(app.comparing && app.differences.is_empty());
    let valid_options = format!("{:?}", app.compare_options);
    assert!(
        app.set_compare_options(CompareOptions {
            ignore_regex: "(".into(),
            ..CompareOptions::default()
        })
        .is_err()
    );
    assert_eq!(format!("{:?}", app.compare_options), valid_options);
    app.editors[0].replace(12..12, "edited ").unwrap();
    pump(app);
    assert!(
        !app.comparing,
        "Selected-line comparisons require reselection after edits"
    );
    assert!(app.compare_selection.is_none());
    assert!(app.compare_pair.is_none());
    assert!(app.compare_rx.is_none() && app.compare_due.is_none());
    assert_eq!(app.compare_leading, [0, 0]);
    assert!(
        app.editors
            .iter()
            .all(|editor| editor.widget().margin_top() == 0)
    );
    assert!(app.note.contains("reselect lines"));
    app.command(COMPARE_CLEAR).unwrap();
    app.editors[0].set_text("A 1\n\nsame\n").unwrap();
    app.editors[1].set_text("a2\nsame\n").unwrap();
    pump(app);
    app.begin_compare(document_index(app, left), document_index(app, right), None)
        .unwrap();
    wait_for_comparison(app);
    assert!(
        app.differences.is_empty(),
        "Whitespace/case/empty-line/regex options affect comparison"
    );
    app.command(COMPARE_CLEAR).unwrap();
    app.editors[0]
        .set_text("start\nmoved\nkeep\nend\n")
        .unwrap();
    app.editors[1]
        .set_text("start\nkeep\nmoved\nend\n")
        .unwrap();
    pump(app);
    app.set_compare_options(CompareOptions::default()).unwrap();
    app.begin_compare(document_index(app, left), document_index(app, right), None)
        .unwrap();
    wait_for_comparison(app);
    assert!((0..4).any(|line| app.editors[0].send(SCI_MARKERGET, line, 0) & (1 << 22) != 0));
    app.set_compare_options(CompareOptions {
        detect_moves: false,
        ..CompareOptions::default()
    })
    .unwrap();
    wait_for_comparison(app);
    assert!((0..4).all(|line| app.editors[0].send(SCI_MARKERGET, line, 0) & (1 << 22) == 0));
    app.command(COMPARE_CLEAR).unwrap();
    let common: String = (0..120)
        .map(|line| format!("common line {line}\n"))
        .collect();
    app.editors[0].set_text(&common).unwrap();
    app.editors[1]
        .set_text(&format!("inserted header\n{common}"))
        .unwrap();
    pump(app);
    app.begin_compare(document_index(app, left), document_index(app, right), None)
        .unwrap();
    wait_for_comparison(app);
    pump(app);
    assert_eq!(
        app.compare_row, 0,
        "The initial viewport starts before leading spacer rows"
    );
    assert_eq!(app.compare_leading, [1, 0]);
    assert_eq!(
        app.editors[0].widget().margin_top(),
        app.editors[0].send(SCI_TEXTHEIGHT, 0, 0) as i32
    );
    wait_for(app, |app| {
        app.editors[0]
            .widget()
            .translate_coordinates(&app.pane_frames[0], 0, 0)
            .unwrap()
            .1
            == app.editors[0].send(SCI_TEXTHEIGHT, 0, 0) as i32
    });
    app.align_compare_row(5);
    pump(app);
    assert_eq!(app.editors[0].widget().margin_top(), 0);
    assert_eq!(app.editors[0].send(SCI_GETFIRSTVISIBLELINE, 0, 0), 4);
    assert_eq!(app.editors[1].send(SCI_GETFIRSTVISIBLELINE, 0, 0), 5);
    assert_eq!(app.editors[0].text().unwrap(), common);
    app.launch_compare().unwrap();
    wait_for_comparison(app);
    pump(app);
    assert_eq!(
        app.compare_row, 5,
        "Applying a refreshed comparison preserves its virtual top row"
    );
    assert_eq!(app.editors[0].send(SCI_GETFIRSTVISIBLELINE, 0, 0), 4);
    let end = app.editors[1].length();
    app.editors[1]
        .replace(end..end, "added at the end\n")
        .unwrap();
    wait_for_comparison(app);
    pump(app);
    assert_eq!(
        app.compare_row, 5,
        "Live edits must not reset comparison scrolling"
    );
    assert_eq!(app.editors[0].send(SCI_GETFIRSTVISIBLELINE, 0, 0), 4);
    app.editors[0].send(SCI_SETFIRSTVISIBLELINE, 0, 0);
    app.event(Event::Updated(0, left)).unwrap();
    pump(app);
    assert_eq!(
        app.compare_row, 0,
        "Returning to the top restores leading spacer rows"
    );
    assert_eq!(app.editors[1].send(SCI_GETFIRSTVISIBLELINE, 0, 0), 0);
    assert_eq!(
        app.editors[0].widget().margin_top(),
        app.editors[0].send(SCI_TEXTHEIGHT, 0, 0) as i32
    );
    let minimum_height = app.window.preferred_height().0;
    app.command(COMPARE_CLEAR).unwrap();
    app.editors[1]
        .set_text(&format!("{}{}", "inserted header\n".repeat(512), common))
        .unwrap();
    pump(app);
    app.begin_compare(document_index(app, left), document_index(app, right), None)
        .unwrap();
    wait_for_comparison(app);
    pump(app);
    assert_eq!(app.compare_leading, [512, 0]);
    assert_eq!(
        app.compare_top,
        [512, 0],
        "Logical spacers must not be truncated with rendered pixels"
    );
    assert_eq!(app.current_compare_row(), 0);
    assert!(app.editors[0].widget().margin_top() < app.pane_frames[0].allocated_height());
    assert!(app.editors[0].widget().allocated_height() > 0);
    assert_eq!(
        app.window.preferred_height().0,
        minimum_height,
        "Large leading gaps must not enlarge the window's minimum height"
    );
    app.launch_compare().unwrap();
    wait_for_comparison(app);
    pump(app);
    assert_eq!(
        app.current_compare_row(),
        0,
        "Clamped pixel margins must not advance the logical viewport on refresh"
    );
    app.command(COMPARE_CLEAR).unwrap();
    assert_eq!(app.editors[0].widget().margin_top(), 0);
    assert_eq!(app.editors[0].send(SCI_GETWRAPMODE, 0, 0), 1);
    assert_eq!(app.editors[0].send(SCI_ANNOTATIONGETLINES, 0, 0), 0);
    app.command(WRAP).unwrap();
    app.command(SPLIT).unwrap();
    app.switch(document_index(app, left)).unwrap();
    text(app, &texts[0]);
    app.set_compare_options(CompareOptions::default()).unwrap();
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text("clipboard comparison\n");
    app.command(COMPARE_CLIPBOARD).unwrap();
    wait_for_comparison(app);
    assert_eq!(app.editors[1].text().unwrap(), "clipboard comparison\n");
    assert!(
        app.documents[app.secondary.unwrap()]
            .snapshot
            .path
            .is_none()
    );
    assert!(app.documents[app.secondary.unwrap()].snapshot.dirty);
    assert_eq!(app.editors[0].text().unwrap(), texts[0]);
    app.command(COMPARE_CLEAR).unwrap();
    app.command(SPLIT).unwrap();
    app.switch(document_index(app, left)).unwrap();
    let path = directory.join("compare-saved.txt");
    app.save_to(&path, true).unwrap();
    app.editor().replace(0..0, "local changes\n").unwrap();
    pump(app);
    let local = app.editor().text().unwrap();
    app.command(COMPARE_SAVED).unwrap();
    wait_for_comparison(app);
    assert_eq!(app.editors[0].text().unwrap(), local);
    assert_eq!(app.editors[1].text().unwrap(), texts[0]);
    assert_eq!(fs::read_to_string(&path).unwrap(), texts[0]);
    let pair = app.compare_pair;
    app.event(Event::PinTab(left)).unwrap();
    app.event(Event::MoveTab(left, 0)).unwrap();
    assert_eq!(app.compare_pair, pair);
    assert_eq!(app.documents[app.primary].snapshot.id, left);
    app.editors[0].replace(0..0, "more ").unwrap();
    wait_for_comparison(app);
    assert_eq!(
        app.compare_pair, pair,
        "Live refresh keeps the original comparison pair"
    );
    app.command(COMPARE_CLEAR).unwrap();
    app.command(SPLIT).unwrap();
    while let Some(index) = app
        .documents
        .iter()
        .position(|doc| doc.snapshot.id != original)
    {
        app.remove_document(index).unwrap();
    }
    app.switch(document_index(app, original)).unwrap();
    pump(app);
}

#[test]
fn gtk_workflows_preserve_editing_features_and_recovery() {
    gtk::init().expect("Run GUI tests in a desktop session or with xvfb-run");
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
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
        .downcast::<gtk::EventBox>()
        .unwrap()
        .child()
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
    exercise_tab_order(&mut app);
    exercise_external_reload(&mut app, &directory);
    exercise_folder_search(&mut app, &directory);
    exercise_compare_modes(&mut app, &directory);

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
    app.command(TAB_PIN).unwrap();
    app.command(MONITOR_FILES).unwrap();
    app.set_compare_options(CompareOptions {
        ignore_case: true,
        ..CompareOptions::default()
    })
    .unwrap();
    let tab_order: Vec<_> = app
        .documents
        .iter()
        .map(|doc| (doc.snapshot.title.clone(), doc.snapshot.pinned))
        .collect();
    let recovered_count = app.documents.len();
    app.close().unwrap();
    drop(app);
    EVENTS.with(|events| events.borrow_mut().clear());
    let mut app = App::new(&state).unwrap();
    app.window.show_all();
    app.layout();
    pump(&mut app);
    assert_eq!(app.documents.len(), recovered_count);
    assert_eq!(
        app.documents
            .iter()
            .map(|doc| (doc.snapshot.title.clone(), doc.snapshot.pinned))
            .collect::<Vec<_>>(),
        tab_order
    );
    assert!(app.documents[app.index()].snapshot.pinned);
    assert!(!app.monitor_files);
    assert!(app.compare_options.ignore_case);
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
