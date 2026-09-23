use crate::{
    comparison::{self, CompareOptions, Comparison},
    completion::{self, Api},
    core::{
        self, CaseOp, Difference, EditorFont, Encoding, Eol, JsonNode, LineOp, Result, Search,
        SearchMode,
    },
    editor::{self, DocumentHandle, Editor, Palette, sci::*},
    folder_search::{self, FolderOptions},
    languages::{self, Language},
    monitor::{self, Monitor},
    search_results::{self, Input as SearchInput, Link as ResultLink, Results as SearchResults},
    session::{self, DocumentSnapshot, RecoveryWorker, Session},
    symbols::ShowSymbols,
    toolbar,
    udl::{self, Highlight},
};
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const NEW: usize = 1001;
const OPEN: usize = 1002;
const SAVE: usize = 1003;
const SAVE_AS: usize = 1004;
const CLOSE: usize = 1005;
const EXIT: usize = 1006;
const UNDO: usize = 1010;
const REDO: usize = 1011;
const CUT: usize = 1012;
const COPY: usize = 1013;
const PASTE: usize = 1014;
const SELECT_ALL: usize = 1015;
const UPPER: usize = 1020;
const LOWER: usize = 1021;
const SORT: usize = 1022;
const SORT_DESC: usize = 1023;
const UNIQUE: usize = 1024;
const TRIM: usize = 1025;
const REMOVE_EMPTY: usize = 1026;
const DUPLICATE: usize = 1027;
const DELETE_LINE: usize = 1028;
const ADD_NEXT: usize = 1029;
const SELECT_MATCHES: usize = 1030;
const FIND: usize = 1040;
const FIND_NEXT: usize = 1041;
const FIND_PREVIOUS: usize = 1042;
const REPLACE: usize = 1043;
const REPLACE_ALL: usize = 1044;
const SPLIT: usize = 1050;
const MAP: usize = 1051;
const WRAP: usize = 1052;
const ZOOM_RESET: usize = 1053;
const EDITOR_FONT: usize = 1054;
const MONITOR_FILES: usize = 1055;
const THEME_SYSTEM: usize = 1060;
const THEME_LIGHT: usize = 1061;
const THEME_DARK: usize = 1062;
const COMPARE: usize = 1070;
const DIFF_NEXT: usize = 1071;
const DIFF_PREVIOUS: usize = 1072;
const COMPARE_CLEAR: usize = 1073;
const COMPARE_SETTINGS: usize = 1074;
const COMPARE_SELECTION: usize = 1075;
const COMPARE_CLIPBOARD: usize = 1076;
const COMPARE_SAVED: usize = 1077;
const JSON_FORMAT: usize = 1080;
const JSON_COMPACT: usize = 1081;
const JSON_TREE: usize = 1082;
const JSON_REFRESH: usize = 1083;
const EOL_CRLF: usize = 1090;
const EOL_LF: usize = 1091;
const EOL_CR: usize = 1092;
const ABOUT: usize = 1150;
const SEARCH_CLOSE: usize = 1160;
const COMPLETE: usize = 1161;
const FIND_ALL_CURRENT: usize = 1170;
const FIND_ALL_OPEN: usize = 1171;
const RESULTS_TOGGLE: usize = 1172;
const RESULTS_NEXT: usize = 1173;
const RESULTS_PREVIOUS: usize = 1174;
const RESULTS_CLEAR: usize = 1175;
const RESULTS_CLOSE: usize = 1176;
const RESULTS_CANCEL: usize = 1177;
const RESULTS_ACTIVATE: usize = 1178;
const FIND_FILES: usize = 1179;
const FIND_FILES_RUN: usize = 1180;
const FIND_FOLDER: usize = 1181;
const TAB_PIN: usize = 1182;
const TAB_LEFT: usize = 1183;
const TAB_RIGHT: usize = 1184;
const SYMBOL_SPACE: usize = 1300;
const SYMBOL_EOL: usize = 1301;
const SYMBOL_NONPRINTING: usize = 1302;
const SYMBOL_CONTROLS: usize = 1303;
const SYMBOL_ALL: usize = 1304;
const SYMBOL_INDENT: usize = 1305;
const SYMBOL_WRAP: usize = 1306;
const TITLE_CASE: usize = 1200;
const SENTENCE_CASE: usize = 1201;
const INVERT_CASE: usize = 1202;
const SORT_IGNORE_CASE: usize = 1210;
const SORT_DESC_IGNORE_CASE: usize = 1211;
const SORT_NATURAL: usize = 1212;
const SORT_NUMERIC: usize = 1213;
const SORT_NUMERIC_DESC: usize = 1214;
const SORT_NUMERIC_COMMA: usize = 1215;
const REVERSE_LINES: usize = 1216;
const UNIQUE_ADJACENT: usize = 1217;
const TRIM_START: usize = 1218;
const TRIM_BOTH: usize = 1219;
const REMOVE_EMPTY_ONLY: usize = 1220;
const JOIN_LINES: usize = 1221;
const IMPORT_LANGUAGE: usize = 1400;
const IMPORT_API: usize = 1401;
const REMOVE_LANGUAGE: usize = 1402;
const PARAMETER_HINT: usize = 1403;
const LANGUAGE_BASE: usize = 2000;
const ENCODING_BASE: usize = 5000;
const REOPEN_BASE: usize = 5200;

const TOOLS: &[toolbar::Button] = &[
    toolbar::Button {
        command: NEW,
        name: "New",
        tooltip: "New document (Ctrl+N)",
        icon: "document-new-symbolic",
    },
    toolbar::Button {
        command: OPEN,
        name: "Open",
        tooltip: "Open file (Ctrl+O)",
        icon: "document-open-symbolic",
    },
    toolbar::Button {
        command: SAVE,
        name: "Save",
        tooltip: "Save document (Ctrl+S)",
        icon: "document-save-symbolic",
    },
    toolbar::Button {
        command: FIND,
        name: "Find",
        tooltip: "Find and replace (Ctrl+F)",
        icon: "edit-find-symbolic",
    },
    toolbar::Button {
        command: SPLIT,
        name: "Split",
        tooltip: "Toggle split view (Ctrl+Alt+Right)",
        icon: "view-dual-symbolic",
    },
    toolbar::Button {
        command: COMPARE,
        name: "Compare",
        tooltip: "Compare active tab with next tab",
        icon: "view-restore-symbolic",
    },
    toolbar::Button {
        command: JSON_TREE,
        name: "JSON tree",
        tooltip: "Toggle JSON tree (Ctrl+Alt+T)",
        icon: "view-list-bullet-symbolic",
    },
    toolbar::Button {
        command: MAP,
        name: "Document map",
        tooltip: "Toggle document map",
        icon: "view-list-symbolic",
    },
];

thread_local! {
    static EVENTS: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
    static TABS_UPDATING: Cell<bool> = const { Cell::new(false) };
    static TREE_UPDATING: Cell<bool> = const { Cell::new(false) };
    static SYMBOLS_UPDATING: Cell<bool> = const { Cell::new(false) };
}

enum Event {
    Command(usize),
    Close,
    Theme,
    Tab(u64),
    CloseTab(u64),
    MoveTab(u64, usize),
    PinTab(u64),
    SplitTab(u64),
    CompareTabs(u64, u64),
    StepTab(u64, bool),
    NextTab(bool),
    OtherPane,
    Escape,
    Tree(usize),
    Focus(usize, u64),
    Changed(usize, u64),
    Style(usize, u64),
    Updated(usize, u64),
    Character(usize, u64, i32),
    Uris(String),
    Map(i32),
    MapScroll(isize),
    CompareScroll(usize, u64, isize),
    ResultActivate(usize),
    Error(String),
}

fn queue(event: Event) {
    EVENTS.with(|events| {
        let mut events = events.borrow_mut();
        if let Event::Updated(pane, id) = &event
            && events
                .iter()
                .any(|old| matches!(old, Event::Updated(p, d) if p == pane && d == id))
        {
            return;
        }
        events.push_back(event);
    });
}

fn empty_tab_strip_press(kind: gdk::EventType, button: u32) -> glib::Propagation {
    if kind == gdk::EventType::DoubleButtonPress && button == 1 {
        queue(Event::Command(NEW));
        glib::Propagation::Stop
    } else {
        glib::Propagation::Proceed
    }
}

fn message(
    parent: Option<&gtk::Window>,
    text: &str,
    kind: gtk::MessageType,
    buttons: &[(&str, gtk::ResponseType)],
) -> gtk::ResponseType {
    let dialog = gtk::MessageDialog::new(
        parent,
        gtk::DialogFlags::MODAL,
        kind,
        gtk::ButtonsType::None,
        text,
    );
    dialog.set_title("rstpd");
    dialog.add_buttons(buttons);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let response = dialog.run();
    dialog.close();
    response
}

pub fn show_error(error: &str) {
    eprintln!("rstpd: {error}");
    if gtk::is_initialized_main_thread() {
        message(
            None,
            error,
            gtk::MessageType::Error,
            &[("_Close", gtk::ResponseType::Close)],
        );
    }
}

fn confirm(window: &gtk::Window, text: &str) -> bool {
    message(
        Some(window),
        text,
        gtk::MessageType::Warning,
        &[
            ("_Cancel", gtk::ResponseType::Cancel),
            ("_Continue", gtk::ResponseType::Yes),
        ],
    ) == gtk::ResponseType::Yes
}

fn canonical_destination(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or("A destination filename is required.")?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::canonicalize(parent)
                .map(|directory| directory.join(name))
                .map_err(|error| format!("{}: {error}", parent.display()))
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn editor_font_description(font: &EditorFont) -> gtk::pango::FontDescription {
    let mut description = gtk::pango::FontDescription::new();
    description.set_family(font.family());
    description.set_size((font.size_hundredths() as i32 * gtk::pango::SCALE + 50) / 100);
    description
}

fn editor_font_from_description(description: &gtk::pango::FontDescription) -> Result<EditorFont> {
    if description.is_size_absolute() {
        return Err("Choose an editor font size in points, not pixels.".into());
    }
    let family = description.family().ok_or("Select a font family.")?;
    let scale = i64::from(gtk::pango::SCALE);
    let size = u32::try_from((i64::from(description.size()) * 100 + scale / 2) / scale)
        .map_err(|_| "Editor font size must be between 4 and 72 points.")?;
    EditorFont::new(family.as_str(), size)
}

fn menu_item(menu: &gtk::Menu, command: usize, label: &str) {
    if command == 0 {
        menu.append(&gtk::SeparatorMenuItem::new());
        return;
    }
    let item = gtk::MenuItem::with_label(label);
    item.connect_activate(move |_| queue(Event::Command(command)));
    menu.append(&item);
}

fn submenu(parent: &impl IsA<gtk::MenuShell>, label: &str) -> gtk::Menu {
    let item = gtk::MenuItem::with_mnemonic(label);
    let menu = gtk::Menu::new();
    item.set_submenu(Some(&menu));
    parent.append(&item);
    menu
}

struct SearchBar {
    container: gtk::Grid,
    query: gtk::Entry,
    replace: gtk::Entry,
    mode: gtk::ComboBoxText,
    case: gtk::CheckButton,
    word: gtk::CheckButton,
    buttons: Vec<(usize, gtk::Button)>,
    folder_controls: gtk::Grid,
    directory: gtk::Entry,
    filters: gtk::Entry,
    recursive: gtk::CheckButton,
    hidden: gtk::CheckButton,
    folder_visible: bool,
    visible: bool,
}

impl SearchBar {
    fn new() -> Self {
        let container = gtk::Grid::new();
        container.set_row_spacing(8);
        container.set_column_spacing(8);
        container.set_margin_start(8);
        container.set_margin_end(8);
        container.set_margin_top(8);
        container.set_margin_bottom(8);
        let query = gtk::Entry::new();
        query.set_placeholder_text(Some("Find text or expression"));
        query.set_tooltip_text(Some("Find text or expression"));
        query.set_max_length(32768);
        query.set_hexpand(true);
        query.set_has_frame(true);
        query.set_width_chars(18);
        query.connect_activate(|_| queue(Event::Command(FIND_NEXT)));
        let replace = gtk::Entry::new();
        replace.set_placeholder_text(Some("Replace with"));
        replace.set_tooltip_text(Some("Replacement text; regex captures use $1 or ${name}"));
        replace.set_max_length(32768);
        replace.set_hexpand(true);
        replace.set_has_frame(true);
        replace.connect_activate(|_| queue(Event::Command(REPLACE)));
        let mode = gtk::ComboBoxText::new();
        for label in ["Normal", "Extended (\\n, \\t)", "Regex ($1 captures)"] {
            mode.append_text(label);
        }
        mode.set_active(Some(0));
        let case = gtk::CheckButton::with_label("Match case");
        let word = gtk::CheckButton::with_label("Whole word");
        for (row, label, target) in [
            (0, "_Find:", query.upcast_ref::<gtk::Widget>()),
            (1, "_Replace:", replace.upcast_ref::<gtk::Widget>()),
            (2, "_Mode:", mode.upcast_ref::<gtk::Widget>()),
        ] {
            let label = gtk::Label::with_mnemonic(label);
            label.set_xalign(0.0);
            label.set_mnemonic_widget(Some(target));
            container.attach(&label, 0, row, 1, 1);
        }
        container.attach(&query, 1, 0, 1, 1);
        container.attach(&replace, 1, 1, 1, 1);
        let options = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        options.pack_start(&mode, false, false, 0);
        options.pack_start(&case, false, false, 0);
        options.pack_start(&word, false, false, 0);
        container.attach(&options, 1, 2, 2, 1);
        let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let replacements = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let all = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        container.attach(&navigation, 2, 0, 1, 1);
        container.attach(&replacements, 2, 1, 1, 1);
        container.attach(&all, 1, 3, 2, 1);
        let mut buttons = Vec::new();
        for (label, command, row) in [
            ("Previous", FIND_PREVIOUS, &navigation),
            ("Next", FIND_NEXT, &navigation),
            ("Close", SEARCH_CLOSE, &navigation),
            ("Replace", REPLACE, &replacements),
            ("Replace all", REPLACE_ALL, &replacements),
            ("Find all: current document", FIND_ALL_CURRENT, &all),
            ("Find all: all open documents", FIND_ALL_OPEN, &all),
        ] {
            let button = action_button(label, command);
            row.pack_start(&button, false, false, 0);
            buttons.push((command, button));
        }
        let folder_controls = gtk::Grid::new();
        folder_controls.set_column_spacing(8);
        folder_controls.set_row_spacing(8);
        let directory = gtk::Entry::new();
        directory.set_hexpand(true);
        directory.set_placeholder_text(Some("Folder to search"));
        directory.set_tooltip_text(Some("Search files on disk, not unsaved tab contents"));
        let filters = gtk::Entry::new();
        filters.set_text("*");
        filters.set_tooltip_text(Some(
            "File glob filters, for example *.rs;*.txt;!generated*",
        ));
        let recursive = gtk::CheckButton::with_label("Include subfolders");
        recursive.set_active(true);
        let hidden = gtk::CheckButton::with_label("Include hidden files");
        for (row, caption, field) in [(0, "_Folder:", &directory), (1, "File _filters:", &filters)]
        {
            let label = gtk::Label::with_mnemonic(caption);
            label.set_xalign(0.0);
            label.set_mnemonic_widget(Some(field));
            folder_controls.attach(&label, 0, row, 1, 1);
            folder_controls.attach(field, 1, row, 1, 1);
        }
        for (row, label, command) in [
            (0, "Choose folder...", FIND_FOLDER),
            (1, "Find in files", FIND_FILES_RUN),
        ] {
            let button = action_button(label, command);
            folder_controls.attach(&button, 2, row, 1, 1);
            buttons.push((command, button));
        }
        let options = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        options.pack_start(&recursive, false, false, 0);
        options.pack_start(&hidden, false, false, 0);
        folder_controls.attach(&options, 1, 2, 2, 1);
        container.attach(&folder_controls, 0, 4, 3, 1);
        Self {
            container,
            query,
            replace,
            mode,
            case,
            word,
            buttons,
            folder_controls,
            directory,
            filters,
            recursive,
            hidden,
            folder_visible: false,
            visible: false,
        }
    }
}

fn action_button(label: &str, command: usize) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_relief(gtk::ReliefStyle::Normal);
    button.set_tooltip_text(Some(label));
    button.connect_clicked(move |_| queue(Event::Command(command)));
    button
}

struct SearchTask {
    rx: mpsc::Receiver<Result<SearchResults>>,
    cancelled: Arc<AtomicBool>,
    discard: bool,
}
impl Drop for SearchTask {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

struct ResultPanel {
    container: gtk::Box,
    title: gtk::Label,
    editor: Editor,
    buttons: Vec<(usize, gtk::Button)>,
    visible: bool,
    data: Option<SearchResults>,
    links: Vec<ResultLink>,
    current: Option<usize>,
    job: Option<SearchTask>,
}
impl ResultPanel {
    fn new() -> Result<Self> {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.set_margin_start(8);
        header.set_margin_end(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        let title = gtk::Label::new(Some("Search results"));
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_width_chars(16);
        title.set_max_width_chars(40);
        header.pack_start(&title, true, true, 0);
        let mut buttons = Vec::new();
        for (label, command) in [
            ("Previous", RESULTS_PREVIOUS),
            ("Next", RESULTS_NEXT),
            ("Cancel", RESULTS_CANCEL),
            ("Clear", RESULTS_CLEAR),
            ("Close", RESULTS_CLOSE),
        ] {
            let button = action_button(label, command);
            if matches!(command, RESULTS_PREVIOUS | RESULTS_NEXT | RESULTS_CANCEL) {
                button.set_sensitive(false);
            }
            header.pack_start(&button, false, false, 0);
            buttons.push((command, button));
        }
        container.pack_start(&header, false, false, 0);
        let editor = Editor::new()?;
        editor.widget().set_size_request(-1, 90);
        editor.widget().drag_dest_unset();
        editor.set_read_only_text("Use Find All to search the current document or all open tabs.\nDouble-click a result or press Enter to navigate. F4 / Shift+F4: next / previous match.\n")?;
        editor.connect_notify(|notification| {
            if notification.code == SCN_DOUBLECLICK && notification.position >= 0 {
                queue(Event::ResultActivate(notification.position as usize));
            }
        });
        editor.widget().connect_key_press_event(|_, event| {
            if !event
                .state()
                .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::MOD1_MASK)
            {
                let command = match event.keyval() {
                    gdk::keys::constants::Return | gdk::keys::constants::KP_Enter => {
                        Some(RESULTS_ACTIVATE)
                    }
                    gdk::keys::constants::Escape => Some(RESULTS_CLOSE),
                    _ => None,
                };
                if let Some(command) = command {
                    queue(Event::Command(command));
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        container.pack_start(editor.widget(), true, true, 0);
        Ok(Self {
            container,
            title,
            editor,
            buttons,
            visible: false,
            data: None,
            links: Vec::new(),
            current: None,
            job: None,
        })
    }
    fn has_focus(&self) -> bool {
        self.editor.widget().has_focus()
    }
}

struct Document {
    handle: DocumentHandle,
    snapshot: DocumentSnapshot,
    language: usize,
    base_dirty: bool,
    metadata_dirty: bool,
    revision: u64,
    styled_revision: Option<u64>,
    last_edit: Instant,
}

struct CompareResult {
    left: u64,
    right: u64,
    left_rev: u64,
    right_rev: u64,
    generation: u64,
    options: String,
    result: Result<Comparison>,
}
struct JsonResult {
    document: u64,
    revision: u64,
    result: Result<Vec<JsonNode>>,
}
struct HighlightResult {
    document: u64,
    revision: u64,
    language: String,
    result: Result<Highlight>,
}

struct App {
    window: gtk::Window,
    menu: gtk::MenuBar,
    tabs: gtk::Notebook,
    tab_width: Rc<Cell<i32>>,
    status: gtk::Label,
    search: SearchBar,
    content: gtk::Paned,
    results: ResultPanel,
    tree: gtk::TreeView,
    tree_store: gtk::TreeStore,
    tree_scroll: gtk::ScrolledWindow,
    map_box: gtk::EventBox,
    editors: [Editor; 2],
    pane_frames: [gtk::Overlay; 2],
    pane_ids: [Rc<Cell<u64>>; 2],
    scratch: Editor,
    map: Editor,
    documents: Vec<Document>,
    languages: Vec<Language>,
    completion_api: Vec<Api>,
    primary: usize,
    secondary: Option<usize>,
    focused: usize,
    next_id: u64,
    palette: Palette,
    theme: String,
    editor_font: EditorFont,
    show_symbols: ShowSymbols,
    symbol_items: Vec<(usize, gtk::CheckMenuItem)>,
    monitor_item: Option<gtk::CheckMenuItem>,
    monitor_files: bool,
    monitor: Monitor,
    monitor_busy: bool,
    last_monitor: Instant,
    desktop_settings: Option<gio::Settings>,
    fallback_dark: bool,
    map_visible: bool,
    tree_visible: bool,
    wrap: bool,
    json_nodes: Vec<JsonNode>,
    json_document: Option<u64>,
    json_handles: Vec<gtk::TreeIter>,
    json_due: Option<Instant>,
    json_rx: Option<mpsc::Receiver<JsonResult>>,
    highlight_rx: Option<mpsc::Receiver<HighlightResult>>,
    differences: Vec<Difference>,
    difference: usize,
    comparing: bool,
    compare_options: CompareOptions,
    compare_generation: u64,
    compare_pair: Option<[u64; 2]>,
    compare_selection: Option<[std::ops::Range<usize>; 2]>,
    compare_leading: [usize; 2],
    compare_top: [usize; 2],
    compare_native_row: [usize; 2],
    compare_row: usize,
    compare_aligned: bool,
    compare_rx: Option<mpsc::Receiver<CompareResult>>,
    compare_due: Option<Instant>,
    compare_jump: bool,
    revision: u64,
    recovered_revision: u64,
    recovery_busy: bool,
    recovery: RecoveryWorker,
    last_autosave: Instant,
    recovery_error: Option<String>,
    note: String,
    last_zero_match: Option<(u64, usize, String)>,
    exiting: bool,
    _lock: session::SessionLock,
}

impl App {
    fn new(directory: &Path) -> Result<Self> {
        let lock = session::lock_directory(directory)?;
        let recovery_path = directory.join("session.json");
        let session = session::load(&recovery_path)?;
        let settings = gtk::Settings::default().ok_or("GTK settings are unavailable.")?;
        let fallback_dark = settings.property::<bool>("gtk-application-prefer-dark-theme")
            || settings
                .property::<Option<String>>("gtk-theme-name")
                .is_some_and(|name| name.ends_with("-dark"));
        let desktop_settings = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
            .filter(|schema| schema.has_key("color-scheme"))
            .map(|schema| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None));
        if let Some(settings) = &desktop_settings {
            settings.connect_changed(Some("color-scheme"), |_, _| queue(Event::Theme));
        }
        settings.set_property("gtk-theme-name", "Adwaita");
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("rstpd");
        window.set_icon_name(Some("text-editor"));
        window.set_default_size(1280, 840);
        window.set_size_request(780, 480);
        window.connect_delete_event(|_, _| {
            queue(Event::Close);
            glib::Propagation::Stop
        });
        window.connect_key_press_event(|_, key| keyboard(key));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&root);
        let menu = gtk::MenuBar::new();
        root.pack_start(&menu, false, false, 0);
        root.pack_start(
            &toolbar::build(TOOLS, |id| queue(Event::Command(id))),
            false,
            false,
            0,
        );
        let tabs = gtk::Notebook::new();
        tabs.set_scrollable(true);
        tabs.set_show_border(false);
        tabs.set_can_focus(false);
        tabs.set_hexpand(false);
        tabs.connect_switch_page(move |_, page, _| {
            if !TABS_UPDATING.with(Cell::get)
                && let Ok(id) = page.widget_name().parse::<u64>()
            {
                queue(Event::Tab(id));
            }
        });
        tabs.connect_page_reordered(|_, page, position| {
            if !TABS_UPDATING.with(Cell::get)
                && let Ok(id) = page.widget_name().parse::<u64>()
            {
                queue(Event::MoveTab(id, position as usize));
            }
        });
        let empty_strip = gtk::EventBox::new();
        empty_strip.set_visible_window(false);
        empty_strip.set_size_request(48, -1);
        empty_strip.set_tooltip_text(Some(
            "Double-click this empty tab-strip area to create a new document",
        ));
        empty_strip.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
        empty_strip.connect_button_press_event(|_, event| {
            empty_tab_strip_press(event.event_type(), event.button())
        });
        let tab_strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_strip.pack_start(&tabs, false, false, 0);
        tab_strip.pack_start(&empty_strip, true, true, 0);
        let tab_width = Rc::new(Cell::new(1));
        let width = tab_width.clone();
        let notebook = tabs.clone();
        let blank = empty_strip.clone();
        tab_strip.connect_size_allocate(move |_, allocation| {
            // Scrollable notebooks request only one tab; retain full headers when they fit.
            let width = width.get().min((allocation.width() - 48).max(1));
            notebook.size_allocate(&gtk::Allocation::new(
                allocation.x(),
                allocation.y(),
                width,
                allocation.height(),
            ));
            blank.size_allocate(&gtk::Allocation::new(
                allocation.x() + width,
                allocation.y(),
                (allocation.width() - width).max(1),
                allocation.height(),
            ));
        });
        root.pack_start(&tab_strip, false, false, 0);
        let search = SearchBar::new();
        root.pack_start(&search.container, false, false, 0);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let content = gtk::Paned::new(gtk::Orientation::Vertical);
        content.set_wide_handle(true);
        content.pack1(&body, true, false);
        root.pack_start(&content, true, true, 0);
        let tree_store = gtk::TreeStore::new(&[String::static_type(), u32::static_type()]);
        let tree = gtk::TreeView::with_model(&tree_store);
        tree.set_headers_visible(false);
        tree.set_enable_tree_lines(true);
        tree.set_tooltip_column(0);
        let column = gtk::TreeViewColumn::new();
        let cell = gtk::CellRendererText::new();
        TreeViewColumnExt::pack_start(&column, &cell, true);
        TreeViewColumnExt::add_attribute(&column, &cell, "text", 0);
        tree.append_column(&column);
        tree.selection().connect_changed(|selection| {
            if !TREE_UPDATING.with(Cell::get)
                && let Some((model, iter)) = selection.selected()
            {
                match model.value(&iter, 1).get::<u32>() {
                    Ok(index) => queue(Event::Tree(index as usize)),
                    Err(error) => queue(Event::Error(format!(
                        "Invalid JSON tree selection: {error}"
                    ))),
                }
            }
        });
        let tree_scroll =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        tree_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        tree_scroll.set_size_request(260, -1);
        tree_scroll.add(&tree);
        body.pack_start(&tree_scroll, false, true, 0);
        let editors = [Editor::new()?, Editor::new()?];
        let pane_ids = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
        for (pane, editor) in editors.iter().enumerate() {
            let id = pane_ids[pane].clone();
            editor.connect_notify(move |n| {
                let document = id.get();
                if document == 0 {
                    return;
                }
                match n.code {
                    SCN_FOCUSIN => queue(Event::Focus(pane, document)),
                    SCN_MODIFIED if n.modification & 3 != 0 => {
                        queue(Event::Changed(pane, document))
                    }
                    SCN_STYLENEEDED => queue(Event::Style(pane, document)),
                    SCN_UPDATEUI | SCN_ZOOM => queue(Event::Updated(pane, document)),
                    SCN_CHARADDED => queue(Event::Character(pane, document, n.ch)),
                    SCN_URIDROPPED => {
                        if let Some(text) = n.text {
                            queue(Event::Uris(text));
                        }
                    }
                    _ => {}
                }
            });
        }
        let split = gtk::Paned::new(gtk::Orientation::Horizontal);
        let pane_frames = [gtk::Overlay::new(), gtk::Overlay::new()];
        for (pane, frame) in pane_frames.iter().enumerate() {
            let background = gtk::EventBox::new();
            background.set_size_request(80, 60);
            background.add_events(gdk::EventMask::SCROLL_MASK | gdk::EventMask::SMOOTH_SCROLL_MASK);
            let id = pane_ids[pane].clone();
            background.connect_scroll_event(move |_, event| {
                let delta = match event.direction() {
                    gdk::ScrollDirection::Up => -3,
                    gdk::ScrollDirection::Down => 3,
                    _ => (event.delta().1 * 3.0).round() as isize,
                };
                queue(Event::CompareScroll(pane, id.get(), delta));
                glib::Propagation::Stop
            });
            frame.add(&background);
            frame.add_overlay(editors[pane].widget());
            // Leading comparison rows must not increase the window's minimum height.
            frame.connect_get_child_position(|frame, child| {
                let height = frame.allocated_height().max(1);
                if child.margin_top() >= height {
                    child.set_margin_top(height - 1);
                }
                Some(gdk::Rectangle::new(
                    0,
                    0,
                    frame.allocated_width().max(1),
                    height,
                ))
            });
        }
        split.pack1(&pane_frames[0], true, false);
        split.pack2(&pane_frames[1], true, false);
        body.pack_start(&split, true, true, 0);
        let scratch = Editor::new()?;
        let map = Editor::new()?;
        map.widget().set_can_focus(false);
        map.widget().drag_dest_unset();
        let map_box = gtk::EventBox::new();
        map_box.set_above_child(true);
        map_box.set_visible_window(false);
        map_box.set_size_request(115, -1);
        map_box.add(map.widget());
        map_box.add_events(
            gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::SMOOTH_SCROLL_MASK,
        );
        map_box.connect_button_press_event(|_, event| {
            if event.button() == 1 {
                queue(Event::Map(event.position().1 as i32));
            }
            glib::Propagation::Stop
        });
        map_box.connect_motion_notify_event(|_, event| {
            if event.state().contains(gdk::ModifierType::BUTTON1_MASK) {
                queue(Event::Map(event.position().1 as i32));
            }
            glib::Propagation::Stop
        });
        map_box.connect_scroll_event(|_, event| {
            let delta = match event.direction() {
                gdk::ScrollDirection::Up => -3,
                gdk::ScrollDirection::Down => 3,
                _ => (event.delta().1 * 3.0).round() as isize,
            };
            queue(Event::MapScroll(delta));
            glib::Propagation::Stop
        });
        body.pack_end(&map_box, false, false, 0);
        let results = ResultPanel::new()?;
        content.pack2(&results.container, false, false);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status.set_margin_start(10);
        status.set_margin_end(10);
        status.set_margin_top(6);
        status.set_margin_bottom(6);
        root.pack_end(&status, false, false, 0);
        window.drag_dest_set(
            gtk::DestDefaults::ALL,
            &[gtk::TargetEntry::new(
                "text/uri-list",
                gtk::TargetFlags::OTHER_APP,
                0,
            )],
            gdk::DragAction::COPY,
        );
        window.connect_drag_data_received(|_, context, _, _, data, _, time| {
            let uris = data.uris();
            let accepted = !uris.is_empty();
            if accepted {
                queue(Event::Uris(
                    uris.iter()
                        .map(|uri| uri.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ));
            }
            context.drag_finish(accepted, false, time);
        });
        let mut languages = languages::catalog(&editor::available_lexers());
        for definition in session.custom_languages {
            languages::add_custom(&mut languages, definition)?;
        }
        let mut app = Self {
            window,
            menu,
            tabs,
            tab_width,
            status,
            search,
            content,
            results,
            tree,
            tree_store,
            tree_scroll,
            map_box,
            editors,
            pane_frames,
            pane_ids,
            scratch,
            map,
            documents: Vec::new(),
            languages,
            completion_api: session.completion_api,
            primary: 0,
            secondary: None,
            focused: 0,
            next_id: 1,
            palette: Palette::new(false),
            theme: session.theme,
            editor_font: session.editor_font,
            show_symbols: session.show_symbols,
            symbol_items: Vec::new(),
            monitor_item: None,
            monitor_files: session.monitor_files,
            monitor: Monitor::new(),
            monitor_busy: false,
            last_monitor: Instant::now(),
            desktop_settings,
            fallback_dark,
            map_visible: false,
            tree_visible: false,
            wrap: false,
            json_nodes: Vec::new(),
            json_document: None,
            json_handles: Vec::new(),
            json_due: None,
            json_rx: None,
            highlight_rx: None,
            differences: Vec::new(),
            difference: 0,
            comparing: false,
            compare_options: session.compare_options,
            compare_generation: 0,
            compare_pair: None,
            compare_selection: None,
            compare_leading: [0; 2],
            compare_top: [0; 2],
            compare_native_row: [0; 2],
            compare_row: 0,
            compare_aligned: false,
            compare_rx: None,
            compare_due: None,
            compare_jump: false,
            revision: 0,
            recovered_revision: 0,
            recovery_busy: false,
            recovery: RecoveryWorker::new(recovery_path),
            last_autosave: Instant::now(),
            recovery_error: None,
            note: String::new(),
            last_zero_match: None,
            exiting: false,
            _lock: lock,
        };
        app.make_menu();
        app.apply_theme();
        for snapshot in session.documents {
            app.add_document(snapshot)?;
        }
        if !app.documents.is_empty() {
            let active = app.documents[session.active.min(app.documents.len() - 1)]
                .snapshot
                .id;
            app.partition_tabs();
            let index = app
                .documents
                .iter()
                .position(|doc| doc.snapshot.id == active)
                .unwrap();
            app.switch(index)?;
        }
        Ok(app)
    }

    fn index(&self) -> usize {
        if self.focused == 1 {
            self.secondary.unwrap_or(self.primary)
        } else {
            self.primary
        }
    }
    fn editor(&self) -> Editor {
        self.editors[self.focused].clone()
    }
    fn effective_wrap(&self) -> bool {
        self.wrap && !(self.comparing && self.compare_options.align)
    }
    fn pane_documents(&self) -> (u64, Option<u64>) {
        (
            self.documents[self.primary].snapshot.id,
            self.secondary
                .map(|index| self.documents[index].snapshot.id),
        )
    }
    fn remap_panes(&mut self, ids: (u64, Option<u64>)) {
        self.primary = self
            .documents
            .iter()
            .position(|doc| doc.snapshot.id == ids.0)
            .unwrap();
        self.secondary = ids
            .1
            .and_then(|id| self.documents.iter().position(|doc| doc.snapshot.id == id));
    }
    fn partition_tabs(&mut self) {
        let ids = self.pane_documents();
        self.documents.sort_by_key(|doc| !doc.snapshot.pinned);
        self.remap_panes(ids);
        self.update_tabs();
        self.touch();
    }
    fn move_tab(&mut self, id: u64, requested: usize) -> Result<()> {
        let Some(from) = self.documents.iter().position(|doc| doc.snapshot.id == id) else {
            return Ok(());
        };
        let pinned: Vec<_> = self
            .documents
            .iter()
            .map(|doc| doc.snapshot.pinned)
            .collect();
        let target = crate::tabs::move_target(&pinned, from, requested.min(pinned.len() - 1))?;
        let ids = self.pane_documents();
        let document = self.documents.remove(from);
        self.documents.insert(target, document);
        self.remap_panes(ids);
        self.update_tabs();
        self.touch();
        Ok(())
    }
    fn pin_tab(&mut self, id: u64) {
        if let Some(doc) = self.documents.iter_mut().find(|doc| doc.snapshot.id == id) {
            doc.snapshot.pinned = !doc.snapshot.pinned;
            self.partition_tabs();
        }
    }
    fn document_dirty(&self, index: usize) -> bool {
        let doc = &self.documents[index];
        self.scratch.attach(&doc.handle);
        doc.base_dirty || doc.metadata_dirty || self.scratch.send(SCI_GETMODIFY, 0, 0) != 0
    }
    fn touch(&mut self) {
        self.revision += 1;
    }
    fn note(&mut self, text: impl Into<String>) {
        self.note = text.into();
        self.update_status();
    }
    fn layout(&self) {
        self.search.container.set_visible(self.search.visible);
        self.search
            .folder_controls
            .set_visible(self.search.folder_visible);
        self.tree_scroll.set_visible(self.tree_visible);
        self.map_box.set_visible(self.map_visible);
        self.editors[1]
            .widget()
            .set_visible(self.secondary.is_some());
        self.pane_frames[1].set_visible(self.secondary.is_some());
        self.results.container.set_visible(self.results.visible);
    }

    fn make_menu(&mut self) {
        self.symbol_items.clear();
        for child in self.menu.children() {
            self.menu.remove(&child);
        }
        let menus: &[(&str, &[(usize, &str)])] = &[
            (
                "_File",
                &[
                    (NEW, "New    Ctrl+N"),
                    (OPEN, "Open...    Ctrl+O"),
                    (SAVE, "Save    Ctrl+S"),
                    (SAVE_AS, "Save as...    Ctrl+Shift+S"),
                    (CLOSE, "Close tab    Ctrl+W"),
                    (TAB_PIN, "Pin / unpin tab"),
                    (TAB_LEFT, "Move tab left    Ctrl+Shift+PageUp"),
                    (TAB_RIGHT, "Move tab right    Ctrl+Shift+PageDown"),
                    (0, ""),
                    (EXIT, "Quit (keep session)"),
                ],
            ),
            (
                "_Edit",
                &[
                    (UNDO, "Undo    Ctrl+Z"),
                    (REDO, "Redo    Ctrl+Y"),
                    (0, ""),
                    (CUT, "Cut    Ctrl+X"),
                    (COPY, "Copy    Ctrl+C"),
                    (PASTE, "Paste    Ctrl+V"),
                    (SELECT_ALL, "Select all    Ctrl+A"),
                    (0, ""),
                    (ADD_NEXT, "Add next occurrence    Ctrl+D"),
                    (SELECT_MATCHES, "Select all occurrences    Ctrl+Shift+L"),
                    (COMPLETE, "Completion    Ctrl+Space"),
                    (PARAMETER_HINT, "Function parameters    Ctrl+Shift+Space"),
                ],
            ),
            (
                "_Search",
                &[
                    (FIND, "Find / replace    Ctrl+F / Ctrl+H"),
                    (FIND_FILES, "Find in files...    Ctrl+Shift+F"),
                    (FIND_NEXT, "Find next    F3"),
                    (FIND_PREVIOUS, "Find previous    Shift+F3"),
                    (REPLACE_ALL, "Replace all"),
                    (
                        FIND_ALL_CURRENT,
                        "Find all in current document    Ctrl+Alt+Enter",
                    ),
                    (
                        FIND_ALL_OPEN,
                        "Find all in all open documents    Ctrl+Shift+Enter",
                    ),
                    (0, ""),
                    (RESULTS_TOGGLE, "Search results panel    Ctrl+Alt+R"),
                    (RESULTS_NEXT, "Next search result    F4"),
                    (RESULTS_PREVIOUS, "Previous search result    Shift+F4"),
                ],
            ),
            (
                "_View",
                &[
                    (SPLIT, "Toggle split view    Ctrl+Alt+Right"),
                    (MAP, "Document map"),
                    (WRAP, "Word wrap"),
                    (ZOOM_RESET, "Reset zoom"),
                    (EDITOR_FONT, "Editor font..."),
                    (0, ""),
                    (THEME_SYSTEM, "Theme: system preference"),
                    (THEME_LIGHT, "Theme: Adwaita light"),
                    (THEME_DARK, "Theme: Adwaita dark"),
                ],
            ),
            (
                "_Tools",
                &[
                    (COMPARE, "Compare active tab with next tab"),
                    (COMPARE_SELECTION, "Compare selected lines in both panes"),
                    (COMPARE_CLIPBOARD, "Compare with clipboard"),
                    (COMPARE_SAVED, "Compare with last-saved file"),
                    (COMPARE_SETTINGS, "Compare options..."),
                    (DIFF_NEXT, "Next difference    F7"),
                    (DIFF_PREVIOUS, "Previous difference    Shift+F7"),
                    (COMPARE_CLEAR, "Clear compare"),
                    (0, ""),
                    (JSON_FORMAT, "Format JSON / JSON5    Ctrl+Alt+J"),
                    (JSON_COMPACT, "Minify JSON / JSON5"),
                    (JSON_TREE, "Toggle live JSON tree    Ctrl+Alt+T"),
                    (JSON_REFRESH, "Refresh JSON tree"),
                ],
            ),
        ];
        for (label, items) in menus {
            let menu = submenu(&self.menu, label);
            for (id, label) in *items {
                menu_item(&menu, *id, label);
            }
            if *label == "_View" {
                let item = gtk::CheckMenuItem::with_label("Automatically reload external changes");
                item.set_active(self.monitor_files);
                item.connect_activate(|_| {
                    if !SYMBOLS_UPDATING.with(Cell::get) {
                        queue(Event::Command(MONITOR_FILES));
                    }
                });
                menu.append(&item);
                self.monitor_item = Some(item);
                let symbols = submenu(&menu, "Show _symbols");
                for (id, label) in [
                    (SYMBOL_SPACE, "Show space and tab"),
                    (SYMBOL_EOL, "Show end of line"),
                    (SYMBOL_NONPRINTING, "Show non-printing characters"),
                    (SYMBOL_CONTROLS, "Show control characters & Unicode EOL"),
                    (SYMBOL_ALL, "Show all characters"),
                    (0, ""),
                    (SYMBOL_INDENT, "Show indent guide"),
                    (SYMBOL_WRAP, "Show wrap symbol"),
                ] {
                    if id == 0 {
                        menu_item(&symbols, 0, "");
                        continue;
                    }
                    let item = gtk::CheckMenuItem::with_label(label);
                    item.connect_activate(move |_| {
                        if !SYMBOLS_UPDATING.with(Cell::get) {
                            queue(Event::Command(id));
                        }
                    });
                    symbols.append(&item);
                    self.symbol_items.push((id, item));
                }
            }
            if *label == "_Edit" {
                let cases = submenu(&menu, "Case conversion");
                for (id, label) in [
                    (UPPER, "UPPERCASE    Ctrl+Shift+U"),
                    (LOWER, "lowercase    Ctrl+U"),
                    (TITLE_CASE, "Title Case"),
                    (SENTENCE_CASE, "Sentence case"),
                    (INVERT_CASE, "Invert case"),
                ] {
                    menu_item(&cases, id, label);
                }
                let lines = submenu(&menu, "Line operations");
                for (id, label) in [
                    (DUPLICATE, "Duplicate selection / line"),
                    (DELETE_LINE, "Delete line"),
                    (SORT, "Sort ascending"),
                    (SORT_DESC, "Sort descending"),
                    (SORT_IGNORE_CASE, "Sort ascending, ignore case"),
                    (SORT_DESC_IGNORE_CASE, "Sort descending, ignore case"),
                    (SORT_NATURAL, "Natural sort (file2 before file10)"),
                    (SORT_NUMERIC, "Numeric sort, decimal point"),
                    (SORT_NUMERIC_DESC, "Numeric sort descending"),
                    (SORT_NUMERIC_COMMA, "Numeric sort, decimal comma"),
                    (REVERSE_LINES, "Reverse line order"),
                    (UNIQUE, "Remove duplicate lines"),
                    (UNIQUE_ADJACENT, "Remove consecutive duplicate lines"),
                    (TRIM, "Trim trailing whitespace"),
                    (TRIM_START, "Trim leading whitespace"),
                    (TRIM_BOTH, "Trim leading and trailing whitespace"),
                    (REMOVE_EMPTY, "Remove empty / whitespace-only lines"),
                    (REMOVE_EMPTY_ONLY, "Remove empty lines only"),
                    (JOIN_LINES, "Join lines"),
                ] {
                    menu_item(&lines, id, label);
                }
            }
        }
        let language_menu = submenu(&self.menu, "_Language");
        if let Some(index) = self
            .languages
            .iter()
            .position(|l| l.name == "Plain text" && l.custom.is_none())
        {
            menu_item(&language_menu, LANGUAGE_BASE + index, "Plain text");
        }
        for group in languages::menu_groups(&self.languages) {
            let group_menu = submenu(&language_menu, &group.label.replace('&', "_"));
            for index in group.indices {
                menu_item(
                    &group_menu,
                    LANGUAGE_BASE + index,
                    &self.languages[index].name,
                );
            }
        }
        for (id, label) in [
            (0, ""),
            (IMPORT_LANGUAGE, "Import user-defined language XML..."),
            (IMPORT_API, "Import completion API for current language..."),
            (REMOVE_LANGUAGE, "Remove current user-defined language"),
        ] {
            menu_item(&language_menu, id, label);
        }
        let encoding_menu = submenu(&self.menu, "E_ncoding");
        let reopen = gtk::Menu::new();
        for (group, encodings) in core::encoding_options().chunks(12).enumerate() {
            let label = format!(
                "{} - {}",
                encodings.first().unwrap().label(),
                encodings.last().unwrap().label()
            );
            let convert = submenu(&encoding_menu, &format!("Convert: {label}"));
            let reload = submenu(&reopen, &label);
            for (offset, encoding) in encodings.iter().enumerate() {
                menu_item(
                    &convert,
                    ENCODING_BASE + group * 12 + offset,
                    encoding.label(),
                );
                menu_item(&reload, REOPEN_BASE + group * 12 + offset, encoding.label());
            }
        }
        menu_item(&encoding_menu, 0, "");
        let item = gtk::MenuItem::with_label("Reopen using encoding (discard edits)...");
        item.set_submenu(Some(&reopen));
        encoding_menu.append(&item);
        menu_item(&encoding_menu, 0, "");
        for (id, label) in [
            (EOL_CRLF, "Line endings: Windows (CRLF)"),
            (EOL_LF, "Line endings: Unix (LF)"),
            (EOL_CR, "Line endings: Macintosh (CR)"),
        ] {
            menu_item(&encoding_menu, id, label);
        }
        let help = submenu(&self.menu, "_Help");
        menu_item(&help, ABOUT, "About / keyboard help");
        self.menu.show_all();
        self.update_symbol_checks();
    }

    fn update_symbol_checks(&self) {
        SYMBOLS_UPDATING.with(|flag| flag.set(true));
        for (id, item) in &self.symbol_items {
            let checked = match *id {
                SYMBOL_SPACE => self.show_symbols.whitespace,
                SYMBOL_EOL => self.show_symbols.eol,
                SYMBOL_NONPRINTING => self.show_symbols.non_printing,
                SYMBOL_CONTROLS => self.show_symbols.controls,
                SYMBOL_ALL => self.show_symbols.all_characters(),
                SYMBOL_INDENT => self.show_symbols.indent_guides,
                SYMBOL_WRAP => self.show_symbols.wrap_markers,
                _ => unreachable!(),
            };
            item.set_active(checked);
        }
        SYMBOLS_UPDATING.with(|flag| flag.set(false));
    }
    fn apply_symbols(&self) {
        for editor in &self.editors {
            editor.show_symbols(self.show_symbols, self.palette);
        }
        self.update_symbol_checks();
    }

    fn apply_theme(&mut self) {
        let dark = match self.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => self
                .desktop_settings
                .as_ref()
                .map(|s| s.string("color-scheme") == "prefer-dark")
                .unwrap_or(self.fallback_dark),
        };
        if let Some(settings) = gtk::Settings::default() {
            settings.set_property("gtk-application-prefer-dark-theme", dark);
        }
        self.palette = Palette::new(dark);
        let context = self.window.style_context();
        for (name, destination) in [
            ("theme_base_color", &mut self.palette.background),
            ("theme_bg_color", &mut self.palette.panel),
            ("theme_text_color", &mut self.palette.text),
            ("insensitive_fg_color", &mut self.palette.muted),
            ("borders", &mut self.palette.border),
            ("link_color", &mut self.palette.accent),
            ("theme_selected_bg_color", &mut self.palette.selection),
        ] {
            if let Some(color) = context.lookup_color(name) {
                *destination = editor::rgb(
                    (color.red() * 255.0).round() as u8,
                    (color.green() * 255.0).round() as u8,
                    (color.blue() * 255.0).round() as u8,
                );
            }
        }
        if !self.documents.is_empty() {
            for (pane, index) in [(0, Some(self.primary)), (1, self.secondary)] {
                if let Some(index) = index {
                    self.editors[pane].theme_with_font(
                        &self.languages[self.documents[index].language],
                        self.palette,
                        &self.editor_font,
                    );
                }
            }
            self.configure_map();
        }
        self.apply_symbols();
        self.theme_results();
    }

    fn choose_editor_font(&self) -> Result<Option<EditorFont>> {
        let dialog = gtk::FontChooserDialog::new(Some("Editor font"), Some(&self.window));
        dialog.set_modal(true);
        dialog.set_level(gtk::FontChooserLevel::FAMILY | gtk::FontChooserLevel::SIZE);
        dialog.set_font_desc(&editor_font_description(&self.editor_font));
        let selected = if dialog.run() == gtk::ResponseType::Ok {
            dialog
                .font_desc()
                .ok_or_else(|| {
                    "Select a font family and a size between 4 and 72 points.".to_owned()
                })
                .and_then(|description| editor_font_from_description(&description))
                .map(Some)
        } else {
            Ok(None)
        };
        dialog.close();
        selected
    }

    fn set_editor_font(&mut self, font: EditorFont) {
        self.editor_font = font;
        for editor in &self.editors {
            editor.send(SCI_SETZOOM, 0, 0);
        }
        self.apply_theme();
        if self.compare_aligned {
            self.align_compare_row(self.compare_row);
        }
        self.touch();
        self.note(format!(
            "Default editor font: {}, {} pt.",
            self.editor_font.family(),
            f64::from(self.editor_font.size_hundredths()) / 100.0
        ));
    }

    fn add_document(&mut self, mut snapshot: DocumentSnapshot) -> Result<()> {
        if self.documents.len() >= 256 {
            return Err("At most 256 tabs can be open.".into());
        }
        let handle = self.scratch.create_document()?;
        let language = self
            .languages
            .iter()
            .position(|l| l.name == snapshot.language)
            .unwrap_or(0);
        self.scratch.attach(&handle);
        self.scratch.set_text(&snapshot.text)?;
        self.scratch
            .send(SCI_SETEOLMODE, snapshot.eol.scintilla(), 0);
        self.scratch.language_with_font(
            &self.languages[language],
            self.palette,
            &self.editor_font,
        )?;
        snapshot.id = self.next_id;
        self.next_id += 1;
        snapshot.text.clear();
        let dirty = snapshot.dirty;
        self.documents.push(Document {
            handle,
            snapshot,
            language,
            base_dirty: dirty,
            metadata_dirty: false,
            revision: 0,
            styled_revision: None,
            last_edit: Instant::now(),
        });
        self.switch(self.documents.len() - 1)?;
        self.touch();
        Ok(())
    }

    fn new_document(&mut self) -> Result<()> {
        self.add_document(DocumentSnapshot {
            id: 0,
            title: format!("Untitled {}", self.next_id),
            path: None,
            text: String::new(),
            encoding: Encoding::Utf8,
            eol: Eol::Lf,
            language: "Plain text".into(),
            dirty: false,
            pinned: false,
            disk_hash: None,
            caret: 0,
        })
    }

    fn switch(&mut self, index: usize) -> Result<()> {
        if index >= self.documents.len() {
            return Err("This tab is no longer open.".into());
        }
        if self.comparing && index != self.index() {
            self.clear_compare();
        }
        if self.focused == 1 && self.secondary.is_some() {
            self.secondary = Some(index);
        } else {
            self.primary = index;
            self.focused = 0;
        }
        self.refresh_views()?;
        self.editor()
            .send(SCI_GOTOPOS, self.documents[index].snapshot.caret, 0);
        self.update_tabs();
        self.update_status();
        self.editor().focus();
        self.touch();
        if self.tree_visible {
            self.schedule_json();
        }
        Ok(())
    }

    fn refresh_views(&mut self) -> Result<()> {
        for (pane, index) in [(0, Some(self.primary)), (1, self.secondary)] {
            if let Some(index) = index {
                let doc = &self.documents[index];
                self.pane_ids[pane].set(doc.snapshot.id);
                self.editors[pane].attach(&doc.handle);
                self.editors[pane].language_with_font(
                    &self.languages[doc.language],
                    self.palette,
                    &self.editor_font,
                )?;
                self.editors[pane].send(SCI_SETEOLMODE, doc.snapshot.eol.scintilla(), 0);
                self.editors[pane].send(SCI_SETWRAPMODE, self.effective_wrap() as usize, 0);
            } else {
                self.pane_ids[pane].set(0);
            }
        }
        self.apply_symbols();
        self.configure_map();
        self.layout();
        Ok(())
    }

    fn split_tab(&mut self, id: u64) -> Result<()> {
        let index = self
            .documents
            .iter()
            .position(|doc| doc.snapshot.id == id)
            .ok_or("This tab is no longer open.")?;
        self.clear_compare();
        let other = 1 - self.focused;
        if other == 1 {
            self.secondary = Some(index);
        } else {
            self.primary = index;
        }
        self.refresh_views()?;
        self.editors[other].send(SCI_GOTOPOS, self.documents[index].snapshot.caret, 0);
        self.update_tabs();
        self.update_status();
        self.editor().focus();
        self.touch();
        Ok(())
    }

    fn compare_tabs(&mut self, current: u64, target: u64) -> Result<()> {
        let left = self
            .documents
            .iter()
            .position(|doc| doc.snapshot.id == current)
            .ok_or("The current document is no longer open.")?;
        let right = self
            .documents
            .iter()
            .position(|doc| doc.snapshot.id == target)
            .ok_or("This tab is no longer open.")?;
        self.begin_compare(left, right, None)
    }

    fn configure_map(&self) {
        if self.documents.is_empty() {
            return;
        }
        let doc = &self.documents[self.index()];
        self.map.attach(&doc.handle);
        self.map.theme_with_font(
            &self.languages[doc.language],
            self.palette,
            &self.editor_font,
        );
        for style in 0..256 {
            self.map.send(SCI_STYLESETSIZEFRACTIONAL, style, 200);
        }
        for margin in 0..5 {
            self.map.send(SCI_SETMARGINWIDTHN, margin, 0);
        }
        for (message, value) in [
            (SCI_SETCARETWIDTH, 0),
            (SCI_SETCARETLINEVISIBLE, 0),
            (SCI_SETHSCROLLBAR, 0),
            (SCI_SETVSCROLLBAR, 0),
            (SCI_SETWRAPMODE, 0),
        ] {
            self.map.send(message, value, 0);
        }
        self.update_map_view();
    }

    fn update_map_view(&self) {
        if !self.map_visible {
            return;
        }
        let editor = self.editor();
        let first = editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0) as usize;
        let count = editor.send(SCI_LINESONSCREEN, 0, 0).max(1) as usize;
        let first_line = editor.send(SCI_DOCLINEFROMVISIBLE, first, 0).max(0) as usize;
        let last_line = editor.send(SCI_DOCLINEFROMVISIBLE, first + count, 0);
        let start = editor.send(SCI_POSITIONFROMLINE, first_line, 0).max(0) as usize;
        let end = if last_line < 0 {
            editor.length()
        } else {
            let position = editor.send(SCI_POSITIONFROMLINE, last_line as usize, 0);
            if position < 0 {
                editor.length()
            } else {
                position as usize
            }
        };
        self.map.send(SCI_SETSEL, start, end as isize);
        self.map.send(SCI_SCROLLCARET, 0, 0);
    }

    fn update_tabs(&self) {
        TABS_UPDATING.with(|flag| flag.set(true));
        while self.tabs.n_pages() > 0 {
            self.tabs.remove_page(Some(0));
        }
        for doc in &self.documents {
            let title = format!(
                "{}{}{}{}",
                if self
                    .secondary
                    .is_some_and(|i| self.documents[i].snapshot.id == doc.snapshot.id)
                {
                    "[R] "
                } else {
                    ""
                },
                if doc.snapshot.pinned { "[Pinned] " } else { "" },
                doc.snapshot.title,
                if doc.snapshot.dirty { " *" } else { "" }
            );
            let label = gtk::Label::new(Some(&title));
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_width_chars(20);
            label.set_max_width_chars(24);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.pack_start(&label, true, true, 0);
            let close =
                gtk::Button::from_icon_name(Some("window-close-symbolic"), gtk::IconSize::Menu);
            close.set_relief(gtk::ReliefStyle::None);
            close.set_can_focus(false);
            close.set_tooltip_text(Some("Close tab"));
            let id = doc.snapshot.id;
            close.connect_clicked(move |_| queue(Event::CloseTab(id)));
            row.pack_end(&close, false, false, 0);
            row.show_all();
            let tab = gtk::EventBox::new();
            tab.set_visible_window(false);
            tab.add(&row);
            let pinned = doc.snapshot.pinned;
            let current = self.documents[self.index()].snapshot.id;
            tab.connect_button_press_event(move |tab, event| {
                if event.button() != 3 {
                    return glib::Propagation::Proceed;
                }
                let menu = gtk::Menu::new();
                menu.set_attach_widget(Some(tab));
                menu.connect_selection_done(|menu| unsafe { menu.destroy() });
                for (label, action) in [
                    (if pinned { "Unpin tab" } else { "Pin tab" }, 0),
                    ("Move tab left", 1),
                    ("Move tab right", 2),
                    ("Open in split view", 3),
                    ("Compare with current view", 4),
                ] {
                    let item = gtk::MenuItem::with_label(label);
                    item.set_sensitive(action != 4 || current != id);
                    item.connect_activate(move |_| {
                        queue(match action {
                            0 => Event::PinTab(id),
                            1 => Event::StepTab(id, true),
                            2 => Event::StepTab(id, false),
                            3 => Event::SplitTab(id),
                            _ => Event::CompareTabs(current, id),
                        })
                    });
                    menu.append(&item);
                }
                menu.show_all();
                menu.popup_at_pointer(Some(event));
                glib::Propagation::Stop
            });
            tab.show_all();
            let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
            page.set_widget_name(&id.to_string());
            self.tabs.append_page(&page, Some(&tab));
            self.tabs.set_tab_reorderable(&page, true);
            page.show();
        }
        self.tabs.set_current_page(Some(self.index() as u32));
        self.tabs.show();
        self.tabs.set_scrollable(false);
        self.tab_width.set(self.tabs.preferred_width().1.max(1));
        self.tabs.set_scrollable(true);
        TABS_UPDATING.with(|flag| flag.set(false));
        if let Some(doc) = self.documents.get(self.index()) {
            self.window.set_title(&format!(
                "{}{} - rstpd",
                doc.snapshot.title,
                if doc.snapshot.dirty { " *" } else { "" }
            ));
        }
    }

    fn update_status(&self) {
        if self.documents.is_empty() {
            return;
        }
        let doc = &self.documents[self.index()];
        let editor = self.editor();
        let line = editor.send(SCI_LINEFROMPOSITION, editor.position(), 0) + 1;
        let column = editor.send(SCI_GETCOLUMN, editor.position(), 0) + 1;
        let characters = match editor.character_counts() {
            Ok(counts) => counts.to_string(),
            Err(error) => format!("Character count unavailable: {error}"),
        };
        let state = if self.recovery_error.is_some() {
            "RECOVERY FAILED"
        } else if self.recovery_busy {
            "Backing up..."
        } else if self.recovered_revision == self.revision {
            "Session backed up"
        } else {
            "Recovery pending"
        };
        let text = format!(
            "Ln {line}, Col {column}   |   {characters}   |   {} selections   |   {}   |   {}   {}   |   {state}   {}",
            editor.send(SCI_GETSELECTIONS, 0, 0),
            self.languages[doc.language].name,
            doc.snapshot.encoding.label(),
            doc.snapshot.eol.label(),
            self.recovery_error.as_deref().unwrap_or(&self.note)
        );
        self.status.set_text(&text);
        self.status.set_tooltip_text(Some(&text));
    }

    fn open_path(&mut self, path: &Path, encoding: Option<&Encoding>) -> Result<()> {
        let path = fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.snapshot.path.as_ref() == Some(&path))
        {
            return self.switch(index);
        }
        let bytes = session::read_bounded(&path, core::MAX_DOCUMENT_BYTES)?;
        let oem = Encoding::Legacy("IBM437".into());
        let fallback = if encoding.is_none()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("nfo"))
            && std::str::from_utf8(&bytes).is_err()
            && !bytes.starts_with(b"\xff\xfe")
            && !bytes.starts_with(b"\xfe\xff")
            && !bytes.starts_with(b"\0\0\xfe\xff")
        {
            Some(&oem)
        } else {
            encoding
        };
        let (text, encoding) = core::decode(&bytes, fallback)?;
        let assumed = matches!(&encoding, Encoding::Legacy(name) if name == "windows-1252");
        let eol = Eol::detect(&text);
        let language = languages::detect(&path, &self.languages);
        self.add_document(DocumentSnapshot {
            id: 0,
            title: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: Some(path),
            text,
            encoding,
            eol,
            language: self.languages[language].name.clone(),
            dirty: false,
            pinned: false,
            disk_hash: Some(session::fingerprint(&bytes)),
            caret: 0,
        })?;
        if assumed {
            self.note("No Unicode BOM / valid UTF-8: opened as Windows-1252; Encoding > Reopen can change this.");
        }
        Ok(())
    }

    fn dialog(&self, save: bool) -> Result<Option<PathBuf>> {
        let dialog = gtk::FileChooserNative::new(
            Some(if save { "Save document" } else { "Open file" }),
            Some(&self.window),
            if save {
                gtk::FileChooserAction::Save
            } else {
                gtk::FileChooserAction::Open
            },
            Some(if save { "_Save" } else { "_Open" }),
            Some("_Cancel"),
        );
        dialog.set_local_only(true);
        dialog.set_do_overwrite_confirmation(true);
        if save {
            let doc = &self.documents[self.index()].snapshot;
            if let Some(path) = &doc.path {
                if let Some(parent) = path.parent() {
                    dialog.set_current_folder(parent);
                }
                if let Some(name) = path.file_name() {
                    dialog.set_current_name(&name.to_string_lossy());
                }
            } else {
                dialog.set_current_name(&doc.title);
            }
        }
        let response = dialog.run();
        let path = dialog.filename();
        dialog.destroy();
        if response == gtk::ResponseType::Accept {
            path.map(Some)
                .ok_or_else(|| "The file chooser did not return a local path.".into())
        } else {
            Ok(None)
        }
    }

    fn save_to(&mut self, path: &Path, save_as: bool) -> Result<()> {
        let index = self.index();
        let canonical = canonical_destination(path)?;
        if self
            .documents
            .iter()
            .enumerate()
            .any(|(i, doc)| i != index && doc.snapshot.path.as_ref() == Some(&canonical))
        {
            return Err(
                "This path is already open in another tab. Save to a different path.".into(),
            );
        }
        let text = self.editor().text()?;
        let bytes = self.documents[index].snapshot.encoding.encode(&text)?;
        session::atomic_write(path, &bytes)?;
        let doc = &mut self.documents[index];
        let was_unnamed = doc.snapshot.path.is_none();
        doc.snapshot.path = Some(
            fs::canonicalize(path)
                .map_err(|e| format!("Saved, but could not resolve its path: {e}"))?,
        );
        doc.snapshot.title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        doc.snapshot.disk_hash = Some(session::fingerprint(&bytes));
        doc.snapshot.dirty = false;
        doc.base_dirty = false;
        doc.metadata_dirty = false;
        if was_unnamed || save_as {
            doc.language = languages::detect(path, &self.languages);
            doc.snapshot.language = self.languages[doc.language].name.clone();
            doc.styled_revision = None;
        }
        self.editor().send(SCI_SETSAVEPOINT, 0, 0);
        self.touch();
        self.refresh_views()?;
        self.update_tabs();
        self.note("Saved.");
        Ok(())
    }

    fn save_document(&mut self, save_as: bool) -> Result<bool> {
        let index = self.index();
        let path = if save_as || self.documents[index].snapshot.path.is_none() {
            let Some(path) = self.dialog(true)? else {
                return Ok(false);
            };
            path
        } else {
            self.documents[index].snapshot.path.clone().unwrap()
        };
        if self.documents[index].snapshot.path.as_ref() == Some(&canonical_destination(&path)?) {
            let current = if path.try_exists().map_err(|e| e.to_string())? {
                Some(session::fingerprint(&session::read_bounded(
                    &path,
                    core::MAX_DOCUMENT_BYTES,
                )?))
            } else {
                None
            };
            if current != self.documents[index].snapshot.disk_hash
                && !confirm(
                    &self.window,
                    "The file changed or was deleted outside rstpd. Overwrite the external version?",
                )
            {
                return Ok(false);
            }
        }
        self.save_to(&path, save_as)?;
        Ok(true)
    }

    fn close_document(&mut self) -> Result<()> {
        let index = self.index();
        if self.document_dirty(index) {
            match message(
                Some(&self.window),
                "Save this document before closing its tab?\n\nDiscard removes its edits and recovery copy. Close the app instead to keep every tab without choosing filenames.",
                gtk::MessageType::Warning,
                &[
                    ("_Cancel", gtk::ResponseType::Cancel),
                    ("_Discard", gtk::ResponseType::No),
                    ("_Save", gtk::ResponseType::Yes),
                ],
            ) {
                gtk::ResponseType::Yes => {
                    if !self.save_document(false)? {
                        return Ok(());
                    }
                }
                gtk::ResponseType::No => {}
                _ => return Ok(()),
            }
        }
        self.remove_document(index)
    }

    fn remove_document(&mut self, index: usize) -> Result<()> {
        self.clear_compare();
        if self.documents.len() == 1 {
            self.new_document()?;
        }
        let closed = self.documents.remove(index);
        if self
            .results
            .data
            .as_ref()
            .is_some_and(|data| data.files.iter().any(|file| file.id == closed.snapshot.id))
        {
            self.results
                .title
                .set_text("A result document was closed - rerun Find All to refresh.");
        }
        self.primary = if self.primary > index {
            self.primary - 1
        } else {
            self.primary.min(self.documents.len() - 1)
        };
        self.secondary = self.secondary.and_then(|i| {
            if i == index {
                None
            } else {
                Some(if i > index { i - 1 } else { i })
            }
        });
        if self.secondary.is_none() {
            self.focused = 0;
            self.editors[1].attach(&self.documents[self.primary].handle);
        }
        self.refresh_views()?;
        self.json_document = None;
        self.json_nodes.clear();
        self.json_handles.clear();
        TREE_UPDATING.with(|flag| flag.set(true));
        self.tree_store.clear();
        TREE_UPDATING.with(|flag| flag.set(false));
        self.touch();
        self.update_tabs();
        self.update_status();
        if self.tree_visible {
            self.schedule_json();
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<Session> {
        let documents = self
            .documents
            .iter()
            .map(|doc| {
                self.scratch.attach(&doc.handle);
                let mut snapshot = doc.snapshot.clone();
                snapshot.text = self.scratch.text()?;
                if self.pane_ids[self.focused].get() == snapshot.id {
                    snapshot.caret = self.editor().position();
                }
                snapshot.dirty = doc.base_dirty
                    || doc.metadata_dirty
                    || self.scratch.send(SCI_GETMODIFY, 0, 0) != 0;
                Ok(snapshot)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Session {
            version: session::SESSION_VERSION,
            documents,
            active: self.index(),
            theme: self.theme.clone(),
            editor_font: self.editor_font.clone(),
            show_symbols: self.show_symbols,
            monitor_files: self.monitor_files,
            compare_options: self.compare_options.clone(),
            custom_languages: self
                .languages
                .iter()
                .filter_map(|l| l.custom.as_deref().cloned())
                .collect(),
            completion_api: self.completion_api.clone(),
        })
    }

    fn poll_monitor(&mut self) {
        while let Ok(changes) = self.monitor.rx.try_recv() {
            self.monitor_busy = false;
            if self.monitor_files {
                for change in changes {
                    if let Err(error) = self.apply_external_change(change) {
                        self.note(format!("External file monitoring: {error}"));
                    }
                }
            }
        }
        if !self.monitor_files
            || self.monitor_busy
            || self.last_monitor.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        self.last_monitor = Instant::now();
        let requests = self
            .documents
            .iter()
            .filter_map(|doc| {
                Some(monitor::Request {
                    id: doc.snapshot.id,
                    path: doc.snapshot.path.clone()?,
                    known_hash: doc.snapshot.disk_hash,
                    encoding: doc.snapshot.encoding.clone(),
                })
            })
            .collect();
        match self.monitor.submit(requests) {
            Ok(()) => self.monitor_busy = true,
            Err(error) => self.note(format!("External file monitoring: {error}")),
        }
    }

    fn apply_external_change(&mut self, change: monitor::Change) -> Result<()> {
        let Some(index) = self.documents.iter().position(|doc| {
            doc.snapshot.id == change.id
                && doc.snapshot.path.as_ref() == Some(&change.path)
                && doc.snapshot.disk_hash == change.baseline_hash
        }) else {
            return Ok(());
        };
        let snapshot = change.result?;
        if self.document_dirty(index)
            && !confirm(
                &self.window,
                &format!(
                    "{} changed outside rstpd.\n\nReload it and discard this tab's unsaved edits? Cancel keeps your edits and the original save-conflict check.",
                    change.path.display()
                ),
            )
        {
            self.note("External change not reloaded; unsaved edits are preserved.");
            return Ok(());
        }
        let views: Vec<_> = self
            .editors
            .iter()
            .enumerate()
            .filter(|(pane, _)| self.pane_ids[*pane].get() == change.id)
            .map(|(pane, editor)| {
                (
                    pane,
                    (0..editor.send(SCI_GETSELECTIONS, 0, 0).max(1) as usize)
                        .map(|selection| {
                            (
                                editor.send(SCI_GETSELECTIONNANCHOR, selection, 0).max(0) as usize,
                                editor.send(SCI_GETSELECTIONNCARET, selection, 0).max(0) as usize,
                            )
                        })
                        .collect::<Vec<_>>(),
                    editor.send(SCI_GETMAINSELECTION, 0, 0).max(0) as usize,
                    editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0).max(0) as usize,
                    editor.send(SCI_GETXOFFSET, 0, 0),
                )
            })
            .collect();
        let boundary = |position: usize| {
            let mut position = position.min(snapshot.text.len());
            while !snapshot.text.is_char_boundary(position) {
                position -= 1;
            }
            position
        };
        self.scratch.attach(&self.documents[index].handle);
        self.scratch.set_text(&snapshot.text)?;
        let doc = &mut self.documents[index];
        doc.snapshot.encoding = snapshot.encoding;
        doc.snapshot.eol = snapshot.eol;
        doc.snapshot.disk_hash = Some(snapshot.hash);
        doc.snapshot.dirty = false;
        doc.snapshot.caret = boundary(doc.snapshot.caret);
        doc.base_dirty = false;
        doc.metadata_dirty = false;
        doc.revision += 1;
        doc.styled_revision = None;
        doc.last_edit = Instant::now();
        let selection_comparison = self.compare_selection.is_some()
            && self
                .compare_pair
                .is_some_and(|pair| pair.contains(&change.id));
        self.invalidate_tools(change.id);
        self.refresh_views()?;
        for (pane, selections, main, first, horizontal) in views {
            let editor = &self.editors[pane];
            for (index, (anchor, caret)) in selections.into_iter().enumerate() {
                editor.send(
                    if index == 0 {
                        SCI_SETSELECTION
                    } else {
                        SCI_ADDSELECTION
                    },
                    boundary(caret),
                    boundary(anchor) as isize,
                );
            }
            editor.send(
                SCI_SETMAINSELECTION,
                main.min(editor.send(SCI_GETSELECTIONS, 0, 0).max(1) as usize - 1),
                0,
            );
            editor.send(SCI_SETFIRSTVISIBLELINE, first, 0);
            editor.send(SCI_SETXOFFSET, horizontal.max(0) as usize, 0);
        }
        self.touch();
        self.update_tabs();
        self.note(format!(
            "Reloaded external changes: {}{}",
            change.path.display(),
            if selection_comparison {
                ". Selection comparison ended; reselect lines to compare."
            } else {
                ""
            }
        ));
        Ok(())
    }

    fn invalidate_tools(&mut self, id: u64) {
        self.last_zero_match = None;
        if self.results.data.as_ref().is_some_and(|data| {
            data.files.iter().any(|file| {
                file.id == id
                    || file.source.as_ref().is_some_and(|source| {
                        self.documents.iter().any(|doc| {
                            doc.snapshot.id == id
                                && doc.snapshot.path.as_ref() == Some(&source.path)
                        })
                    })
            })
        }) {
            self.results
                .title
                .set_text("Document changed - rerun search to refresh its results.");
        }
        if self.json_document == Some(id) {
            self.json_document = None;
        }
        if self.tree_visible && self.documents[self.index()].snapshot.id == id {
            self.schedule_json();
        }
        if self.compare_pair.is_some_and(|pair| pair.contains(&id)) {
            if self.compare_selection.is_some() {
                self.clear_compare();
                self.note("Selection comparison ended after a text change; reselect lines in both panes to compare again.");
                return;
            }
            self.clear_compare_marks();
            self.differences.clear();
            self.compare_due = Some(Instant::now() + Duration::from_millis(350));
        }
    }

    fn search_settings(&self) -> Result<Search> {
        Search::new(
            &self.search.query.text(),
            match self.search.mode.active() {
                Some(1) => SearchMode::Extended,
                Some(2) => SearchMode::Regex,
                _ => SearchMode::Literal,
            },
            self.search.case.is_active(),
            self.search.word.is_active(),
        )
    }

    fn show_search(&mut self) -> Result<()> {
        self.search.visible = true;
        self.last_zero_match = None;
        let selection = self.editor().selection();
        if !selection.is_empty() && selection.len() < 32768 {
            let value = self.editor().range(selection)?;
            if !value.contains(['\r', '\n', '\0']) {
                self.search.query.set_text(&value);
            }
        }
        self.layout();
        self.search.query.grab_focus();
        self.search.query.select_region(0, -1);
        Ok(())
    }

    fn choose_search_folder(&self) {
        let dialog = gtk::FileChooserNative::new(
            Some("Choose search folder"),
            Some(&self.window),
            gtk::FileChooserAction::SelectFolder,
            Some("_Select"),
            Some("_Cancel"),
        );
        dialog.set_local_only(true);
        if !self.search.directory.text().is_empty() {
            dialog.set_current_folder(Path::new(self.search.directory.text().as_str()));
        }
        if dialog.run() == gtk::ResponseType::Accept
            && let Some(path) = dialog.filename()
        {
            self.search.directory.set_text(&path.to_string_lossy());
        }
        dialog.destroy();
    }

    fn start_find_files(&mut self) -> Result<()> {
        if self.results.job.is_some() {
            self.set_results_visible(true);
            self.note("A search is still running. Cancel it before starting another search.");
            return Ok(());
        }
        let query = self.search.query.text().to_string();
        let search = self.search_settings()?;
        let options = FolderOptions {
            directory: PathBuf::from(self.search.directory.text().as_str()),
            filters: self.search.filters.text().to_string(),
            recursive: self.search.recursive.is_active(),
            hidden: self.search.hidden.is_active(),
        };
        options.validate()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        let (tx, rx) = mpsc::channel();
        self.prepare_find_results("Searching files on disk...\n")?;
        std::thread::spawn(move || {
            let _ = tx.send(folder_search::find_in_files(
                &search, query, options, &token,
            ));
        });
        self.results.job = Some(SearchTask {
            rx,
            cancelled,
            discard: false,
        });
        self.result_message("Searching files on disk...");
        self.update_result_buttons();
        Ok(())
    }

    fn tool_text(&self, editor: &Editor) -> Result<String> {
        if editor.length() > core::MAX_TOOL_BYTES {
            return Err("This tool is limited to 16 MiB documents.".into());
        }
        editor.text()
    }

    fn find(&mut self, previous: bool) -> Result<()> {
        let search = self.search_settings()?;
        let editor = self.editor();
        let text = self.tool_text(&editor)?;
        let selection = editor.selection();
        let key = format!(
            "{}:{:?}:{}:{}",
            self.search.query.text(),
            self.search.mode.active(),
            self.search.case.is_active(),
            self.search.word.is_active()
        );
        let document = self.documents[self.index()].snapshot.id;
        let range = if previous {
            let matches = search.matches(&text)?;
            matches
                .iter()
                .rev()
                .find(|r| r.start < selection.start)
                .or(matches.last())
                .cloned()
        } else {
            let mut start = selection.end;
            if selection.is_empty()
                && self.last_zero_match.as_ref() == Some(&(document, start, key.clone()))
            {
                start = editor.send(SCI_POSITIONAFTER, start, 0).max(0) as usize;
                if start == selection.end && start == text.len() {
                    start = 0;
                }
            }
            search.find(&text, start)?
        };
        if let Some(range) = range {
            self.last_zero_match = if range.is_empty() {
                Some((document, range.start, key))
            } else {
                None
            };
            editor.select(range);
            self.note("Match found.");
        } else {
            self.note("No matches.");
        }
        Ok(())
    }

    fn replace(&mut self, all: bool) -> Result<()> {
        let search = self.search_settings()?;
        let editor = self.editor();
        let text = self.tool_text(&editor)?;
        let replacement = self.search.replace.text();
        if all {
            let (out, count) = search.replace_all(&text, &replacement)?;
            if count > 0 {
                editor.replace_all(&out)?;
            }
            self.note(format!("Replaced {count} matches."));
        } else {
            let range = editor.selection();
            if search.find(&text, range.start)? == Some(range.clone()) {
                let output = search.replacement(&text, range.clone(), &replacement)?;
                let end = range.start + output.len();
                editor.replace(range, &output)?;
                editor.select(end..end);
            }
            self.find(false)?;
        }
        Ok(())
    }

    fn theme_results(&self) {
        let editor = &self.results.editor;
        editor.theme(&self.languages[0], self.palette);
        editor.send(SCI_SETMARGINWIDTHN, 0, 0);
        editor.send(SCI_SETMARGINWIDTHN, 1, 0);
        editor.send(SCI_SETWRAPMODE, 0, 0);
        editor.send(SCI_SETCARETLINEVISIBLE, 1, 0);
        editor.send(SCI_STYLESETFORE, 1, self.palette.accent as isize);
        editor.send(SCI_STYLESETBOLD, 1, 1);
        editor.send(SCI_STYLESETBOLD, 2, 1);
        editor.send(SCI_INDICSETSTYLE, search_results::MATCH_INDICATOR, 7);
        editor.send(
            SCI_INDICSETFORE,
            search_results::MATCH_INDICATOR,
            self.palette.accent as isize,
        );
        editor.send(SCI_INDICSETALPHA, search_results::MATCH_INDICATOR, 75);
        editor.send(
            SCI_INDICSETOUTLINEALPHA,
            search_results::MATCH_INDICATOR,
            140,
        );
        editor.send(SCI_INDICSETUNDER, search_results::MATCH_INDICATOR, 1);
    }
    fn result_message(&mut self, message: &str) {
        self.results.title.set_text(message);
        self.results.title.set_tooltip_text(Some(message));
        self.note(message);
    }
    fn set_results_visible(&mut self, visible: bool) {
        let opening = visible && !self.results.visible;
        self.results.visible = visible;
        self.layout();
        if opening {
            self.content
                .set_position((self.content.allocated_height() - 240).max(100));
        }
    }
    fn update_result_buttons(&self) {
        for (command, button) in &self.results.buttons {
            let enabled = match *command {
                RESULTS_NEXT | RESULTS_PREVIOUS => !self.results.links.is_empty(),
                RESULTS_CANCEL => self.results.job.is_some(),
                _ => true,
            };
            button.set_sensitive(enabled);
        }
        for (command, button) in &self.search.buttons {
            if matches!(*command, FIND_ALL_CURRENT | FIND_ALL_OPEN | FIND_FILES_RUN) {
                button.set_sensitive(self.results.job.is_none());
            }
        }
    }
    fn start_find_all(&mut self, all_open: bool) -> Result<()> {
        if self.results.job.is_some() {
            self.set_results_visible(true);
            self.note("Find All is still running. Cancel it before starting another search.");
            return Ok(());
        }
        if self.search.query.text().is_empty() {
            self.show_search()?;
            if self.search.query.text().is_empty() {
                self.note("Enter a search expression, then choose Find All.");
                return Ok(());
            }
        }
        let query = self.search.query.text().to_string();
        let search = self.search_settings()?;
        let mut bytes = 0usize;
        let mut inputs = Vec::new();
        for (index, doc) in self.documents.iter().enumerate() {
            if !all_open && index != self.index() {
                continue;
            }
            self.scratch.attach(&doc.handle);
            let length = self.scratch.length();
            if length > core::MAX_TOOL_BYTES {
                return Err(format!(
                    "{} exceeds the 16 MiB search limit. No documents were skipped or searched.",
                    doc.snapshot.title
                ));
            }
            bytes += length;
            if bytes > search_results::MAX_BATCH_BYTES {
                return Err("Find All is limited to 64 MiB across open documents. Close some tabs or search the current document.".into());
            }
            let title = doc
                .snapshot
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| doc.snapshot.title.clone());
            inputs.push(SearchInput {
                id: doc.snapshot.id,
                revision: doc.revision,
                title,
                text: self.scratch.text()?,
                tab_width: self.scratch.send(SCI_GETTABWIDTH, 0, 0).max(1) as usize,
            });
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let token = cancelled.clone();
        let (tx, rx) = mpsc::channel();
        self.prepare_find_results("Searching open document snapshots...\n")?;
        std::thread::spawn(move || {
            let result = search_results::find_all(&search, query, inputs, &token);
            let _ = tx.send(result);
        });
        self.results.job = Some(SearchTask {
            rx,
            cancelled,
            discard: false,
        });
        self.result_message(if all_open {
            "Searching all open documents..."
        } else {
            "Searching current document..."
        });
        self.update_result_buttons();
        Ok(())
    }
    fn prepare_find_results(&mut self, message: &str) -> Result<()> {
        self.results.editor.set_read_only_text(message)?;
        self.results
            .editor
            .clear_indicator(search_results::MATCH_INDICATOR);
        self.results.data = None;
        self.results.links.clear();
        self.results.current = None;
        self.set_results_visible(true);
        Ok(())
    }
    fn apply_find_results(&mut self, results: SearchResults) -> Result<()> {
        let rendered = results.render();
        self.results.editor.set_read_only_text(&rendered.text)?;
        self.theme_results();
        self.results.editor.highlight(&rendered.highlight)?;
        self.results
            .editor
            .clear_indicator(search_results::MATCH_INDICATOR);
        for range in rendered.emphasis {
            self.results
                .editor
                .indicator(search_results::MATCH_INDICATOR, range)?;
        }
        self.results.links = rendered.links;
        self.results.current = None;
        let stale = results.files.iter().any(|file| {
            file.source.is_none()
                && !self
                    .documents
                    .iter()
                    .any(|doc| doc.snapshot.id == file.id && doc.revision == file.revision)
        });
        let summary = results.summary()
            + if stale {
                " - some documents changed; rerun to navigate those matches."
            } else {
                " - double-click or Enter to navigate."
            };
        self.results.data = Some(results);
        self.result_message(&summary);
        self.update_result_buttons();
        Ok(())
    }
    fn clear_find_results(&mut self) -> Result<()> {
        if let Some(job) = &mut self.results.job {
            job.cancelled.store(true, Ordering::Relaxed);
            job.discard = true;
        }
        self.results
            .editor
            .set_read_only_text("Search results cleared. Run Find All to search again.\n")?;
        self.results
            .editor
            .clear_indicator(search_results::MATCH_INDICATOR);
        self.results.data = None;
        self.results.links.clear();
        self.results.current = None;
        self.result_message("Search results cleared.");
        self.update_result_buttons();
        Ok(())
    }
    fn activate_result(&mut self, index: usize) -> Result<()> {
        let Some(link) = self.results.links.get(index) else {
            return Ok(());
        };
        let Some(results) = &self.results.data else {
            return Ok(());
        };
        let file = &results.files[link.file];
        let range = file.hits[link.hit].range.clone();
        let row = link.row;
        let id = file.id;
        let revision = file.revision;
        let source = file
            .source
            .as_ref()
            .map(|source| (source.path.clone(), source.hash, source.text_hash));
        let document = if let Some((path, hash, text_hash)) = source {
            let bytes = match session::read_bounded(&path, core::MAX_DOCUMENT_BYTES) {
                Ok(bytes) if session::fingerprint(&bytes) == hash => bytes,
                _ => {
                    self.result_message("This file changed or is unavailable since the search. Run Find in files again.");
                    return Ok(());
                }
            };
            let document = if let Some(index) = self
                .documents
                .iter()
                .position(|doc| doc.snapshot.path.as_ref() == Some(&path))
            {
                index
            } else {
                let (_, encoding) = core::decode(&bytes, None)?;
                self.clear_compare();
                self.open_path(&path, Some(&encoding))?;
                self.index()
            };
            self.scratch.attach(&self.documents[document].handle);
            if session::fingerprint(self.scratch.text()?.as_bytes()) != text_hash {
                self.result_message("This document changed since the search (including unsaved edits). Run Find in files again.");
                return Ok(());
            }
            document
        } else {
            let Some(document) = self.documents.iter().position(|doc| doc.snapshot.id == id) else {
                self.result_message("This result's document was closed. Run Find All again.");
                return Ok(());
            };
            if self.documents[document].revision != revision {
                self.result_message("This document changed since the search. Run Find All again before navigating its results.");
                return Ok(());
            }
            document
        };
        if self.comparing && document != self.index() {
            self.clear_compare();
        }
        if document != self.index() {
            self.switch(document)?;
        }
        self.editor().select(range);
        self.editor().focus();
        self.results.current = Some(index);
        let editor = &self.results.editor;
        editor.send(SCI_ENSUREVISIBLEENFORCEPOLICY, row, 0);
        let start = editor.send(SCI_POSITIONFROMLINE, row, 0).max(0) as usize;
        let end = editor
            .send(SCI_GETLINEENDPOSITION, row, 0)
            .max(start as isize) as usize;
        editor.select(start..end);
        self.set_results_visible(true);
        self.result_message(&format!(
            "Search result {} of {} (F4 / Shift+F4)",
            index + 1,
            self.results.links.len()
        ));
        Ok(())
    }
    fn activate_result_position(&mut self, position: usize) -> Result<()> {
        let editor = &self.results.editor;
        let row = editor.send(SCI_LINEFROMPOSITION, position.min(editor.length()), 0) as usize;
        if let Some(index) = self.results.links.iter().position(|link| link.row == row) {
            self.activate_result(index)?;
        } else {
            editor.send(SCI_TOGGLEFOLD, row, 0);
        }
        Ok(())
    }
    fn next_result(&mut self, previous: bool) -> Result<()> {
        let count = self.results.links.len();
        if count == 0 {
            self.set_results_visible(true);
            self.note("No search matches are listed. Run Find All first.");
            return Ok(());
        }
        let index = match (self.results.current, previous) {
            (None, false) => 0,
            (None, true) => count - 1,
            (Some(index), false) => (index + 1) % count,
            (Some(index), true) => (index + count - 1) % count,
        };
        let valid = (0..count)
            .map(|step| {
                if previous {
                    (index + count - step) % count
                } else {
                    (index + step) % count
                }
            })
            .find(|candidate| {
                let Some(results) = &self.results.data else {
                    return false;
                };
                let file = &results.files[self.results.links[*candidate].file];
                file.source.is_some()
                    || self
                        .documents
                        .iter()
                        .any(|doc| doc.snapshot.id == file.id && doc.revision == file.revision)
            });
        self.activate_result(valid.unwrap_or(index))
    }
    fn poll_find_results(&mut self) -> Result<()> {
        let Some(job) = &self.results.job else {
            return Ok(());
        };
        let result = match job.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Find All worker stopped unexpectedly.".into()))
            }
        };
        let Some(result) = result else {
            return Ok(());
        };
        let job = self.results.job.take().expect("active search task");
        let cancelled = job.cancelled.load(Ordering::Relaxed);
        let applied = (|| -> Result<()> {
            if !job.discard {
                match result {
                    Ok(results) if !cancelled => self.apply_find_results(results)?,
                    Ok(_) => {
                        self.results
                            .editor
                            .set_read_only_text("Search cancelled.\n")?;
                        self.result_message("Search cancelled.");
                    }
                    Err(error) => {
                        self.results
                            .editor
                            .set_read_only_text(&format!("Find All: {error}\n"))?;
                        self.result_message(&error);
                    }
                }
            }
            Ok(())
        })();
        self.update_result_buttons();
        applied
    }

    fn clear_compare_alignment(&mut self) {
        self.compare_aligned = false;
        self.compare_leading = [0; 2];
        self.compare_top = [0; 2];
        for (pane, editor) in self.editors.iter().enumerate() {
            editor.widget().set_margin_top(0);
            self.compare_native_row[pane] =
                editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0).max(0) as usize;
        }
    }

    fn current_compare_row(&self) -> usize {
        let visible = self.editor().send(SCI_GETFIRSTVISIBLELINE, 0, 0).max(0) as usize;
        if self.compare_aligned {
            visible
                .saturating_add(self.compare_leading[self.focused])
                .saturating_sub(self.compare_top[self.focused])
        } else {
            self.compare_row.saturating_add_signed(
                visible as isize - self.compare_native_row[self.focused] as isize,
            )
        }
    }

    fn clear_compare_marks(&mut self) {
        let row = if self.compare_jump {
            0
        } else {
            self.current_compare_row()
        };
        for editor in &self.editors {
            editor.clear_diff();
        }
        self.clear_compare_alignment();
        self.compare_row = row;
    }

    fn align_compare_row(&mut self, row: usize) {
        self.compare_row = row;
        for (pane, editor) in self.editors.iter().enumerate() {
            let gap = self.compare_leading[pane].saturating_sub(row);
            self.compare_top[pane] = gap;
            let margin = gap
                .saturating_mul(editor.send(SCI_TEXTHEIGHT, 0, 0).max(1) as usize)
                .min(
                    self.pane_frames[pane]
                        .allocated_height()
                        .saturating_sub(1)
                        .max(0) as usize,
                ) as i32;
            if editor.widget().margin_top() != margin {
                editor.widget().set_margin_top(margin);
            }
            let first = row.saturating_sub(self.compare_leading[pane]);
            if editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0) != first as isize {
                editor.send(SCI_SETFIRSTVISIBLELINE, first, 0);
            }
            self.compare_native_row[pane] =
                editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0).max(0) as usize;
        }
    }

    fn clear_compare(&mut self) {
        for doc in &self.documents {
            self.scratch.attach(&doc.handle);
            self.scratch.clear_diff();
        }
        for editor in &self.editors {
            editor.clear_diff();
            editor.send(SCI_SETWRAPMODE, self.wrap as usize, 0);
        }
        self.differences.clear();
        self.clear_compare_alignment();
        self.compare_row = 0;
        self.comparing = false;
        self.compare_generation += 1;
        self.compare_pair = None;
        self.compare_selection = None;
        self.compare_rx = None;
        self.compare_due = None;
        self.compare_jump = false;
    }

    fn start_compare(&mut self) -> Result<()> {
        if self.documents.len() < 2 {
            return Err("Open two documents in separate tabs to compare.".into());
        }
        let left = self.index();
        self.begin_compare(left, (left + 1) % self.documents.len(), None)
    }

    fn begin_compare(
        &mut self,
        left: usize,
        right: usize,
        selection: Option<[std::ops::Range<usize>; 2]>,
    ) -> Result<()> {
        if left == right || left >= self.documents.len() || right >= self.documents.len() {
            return Err("Choose two different documents to compare.".into());
        }
        self.compare_options.validate()?;
        self.clear_compare();
        self.primary = left;
        self.focused = 0;
        self.secondary = Some(right);
        self.compare_pair = Some([
            self.documents[left].snapshot.id,
            self.documents[right].snapshot.id,
        ]);
        self.compare_selection = selection;
        self.comparing = true;
        self.refresh_views()?;
        self.update_tabs();
        self.editor().focus();
        self.compare_jump = true;
        self.launch_compare()
    }

    fn selected_lines(editor: &Editor) -> Result<std::ops::Range<usize>> {
        let range = editor.selection();
        if range.is_empty() {
            return Err("Select text in each of the two editing panes first. Selections expand to complete lines.".into());
        }
        let first = editor.send(SCI_LINEFROMPOSITION, range.start, 0).max(0) as usize;
        let last_position = editor.send(SCI_POSITIONBEFORE, range.end, 0).max(0) as usize;
        let last = editor.send(SCI_LINEFROMPOSITION, last_position, 0).max(0) as usize;
        Ok(first..last + 1)
    }

    fn compare_selected_lines(&mut self) -> Result<()> {
        let right = self
            .secondary
            .ok_or("Open the split view and select lines in two different documents first.")?;
        let selection = [
            Self::selected_lines(&self.editors[0])?,
            Self::selected_lines(&self.editors[1])?,
        ];
        self.begin_compare(self.primary, right, Some(selection))
    }

    fn compare_snapshot(&mut self, clipboard: bool) -> Result<()> {
        let left = self.index();
        let doc = &self.documents[left].snapshot;
        let (text, title) = if clipboard {
            let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
            let text = clipboard
                .wait_for_text()
                .ok_or("The clipboard does not contain text.")?;
            (text.to_string(), "Clipboard snapshot".to_owned())
        } else {
            let path = doc.path.as_ref().ok_or(
                "Save this document to a file before comparing with its last-saved content.",
            )?;
            let bytes = session::read_bounded(path, core::MAX_DOCUMENT_BYTES)?;
            let encoding = (!self.documents[left].metadata_dirty
                && doc.disk_hash == Some(session::fingerprint(&bytes)))
            .then_some(&doc.encoding);
            let (text, _) = core::decode(&bytes, encoding)?;
            (text, format!("{} (last saved)", doc.title))
        };
        if text.len() > core::MAX_TOOL_BYTES {
            return Err("Comparison snapshots are limited to 16 MiB.".into());
        }
        let language = doc.language.clone();
        let left_id = doc.id;
        self.clear_compare();
        self.focused = 0;
        self.add_document(DocumentSnapshot {
            id: 0,
            title,
            path: None,
            eol: Eol::detect(&text),
            text,
            encoding: Encoding::Utf8,
            language,
            dirty: true,
            pinned: false,
            disk_hash: None,
            caret: 0,
        })?;
        let right = self.index();
        let left = self
            .documents
            .iter()
            .position(|doc| doc.snapshot.id == left_id)
            .unwrap();
        self.begin_compare(left, right, None)
    }

    fn choose_compare_options(&mut self) -> Result<()> {
        let dialog = gtk::Dialog::with_buttons(
            Some("Compare options"),
            Some(&self.window),
            gtk::DialogFlags::MODAL,
            &[
                ("_Cancel", gtk::ResponseType::Cancel),
                ("_Apply", gtk::ResponseType::Ok),
            ],
        );
        let content = dialog.content_area();
        content.set_spacing(8);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        let mut checks = Vec::new();
        for (label, value) in [
            ("Ignore whitespace", self.compare_options.ignore_whitespace),
            ("Ignore case", self.compare_options.ignore_case),
            (
                "Ignore empty lines",
                self.compare_options.ignore_empty_lines,
            ),
            ("Detect moved lines", self.compare_options.detect_moves),
            (
                "Align corresponding lines (temporarily disable wrapping)",
                self.compare_options.align,
            ),
        ] {
            let check = gtk::CheckButton::with_label(label);
            check.set_active(value);
            content.pack_start(&check, false, false, 0);
            checks.push(check);
        }
        let label = gtk::Label::new(Some("Ignore regex matches (empty disables):"));
        label.set_xalign(0.0);
        content.pack_start(&label, false, false, 0);
        let regex = gtk::Entry::new();
        regex.set_text(&self.compare_options.ignore_regex);
        regex.set_max_length(32768);
        content.pack_start(&regex, false, false, 0);
        dialog.show_all();
        let accepted = dialog.run() == gtk::ResponseType::Ok;
        let options = CompareOptions {
            ignore_whitespace: checks[0].is_active(),
            ignore_case: checks[1].is_active(),
            ignore_empty_lines: checks[2].is_active(),
            detect_moves: checks[3].is_active(),
            align: checks[4].is_active(),
            ignore_regex: regex.text().to_string(),
        };
        dialog.close();
        if accepted {
            self.set_compare_options(options)?;
        }
        Ok(())
    }

    fn set_compare_options(&mut self, options: CompareOptions) -> Result<()> {
        options.validate()?;
        self.compare_options = options;
        self.compare_generation += 1;
        self.compare_rx = None;
        if self.comparing {
            self.clear_compare_marks();
            self.differences.clear();
            self.compare_due = Some(Instant::now());
        }
        for editor in &self.editors {
            editor.send(SCI_SETWRAPMODE, self.effective_wrap() as usize, 0);
        }
        self.touch();
        self.note("Comparison options updated.");
        Ok(())
    }

    fn launch_compare(&mut self) -> Result<()> {
        if self.compare_rx.is_some() || !self.comparing {
            return Ok(());
        }
        let Some(right_index) = self.secondary else {
            return Ok(());
        };
        let left = &self.documents[self.primary];
        let right = &self.documents[right_index];
        let (left_id, right_id, left_rev, right_rev) = (
            left.snapshot.id,
            right.snapshot.id,
            left.revision,
            right.revision,
        );
        let mut offsets = [(0, 0); 2];
        let mut texts = Vec::new();
        for (pane, editor) in self.editors.iter().enumerate() {
            let text = if let Some(selection) = &self.compare_selection {
                let lines = &selection[pane];
                let line_count = editor.send(SCI_GETLINECOUNT, 0, 0).max(1) as usize;
                let first = lines.start.min(line_count - 1);
                let start = editor.send(SCI_POSITIONFROMLINE, first, 0).max(0) as usize;
                let end = editor.send(SCI_POSITIONFROMLINE, lines.end.min(line_count), 0);
                let end = if end < 0 {
                    editor.length()
                } else {
                    end as usize
                };
                if end.saturating_sub(start) > core::MAX_TOOL_BYTES {
                    return Err("Selected comparison lines exceed 16 MiB.".into());
                }
                offsets[pane] = (start, first);
                editor.range(start..end.max(start))?
            } else {
                self.tool_text(editor)?
            };
            texts.push(text);
        }
        let options = self.compare_options.clone();
        let generation = self.compare_generation;
        let options_key = format!("{options:?}");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = comparison::compare(&texts[0], &texts[1], &options).map(|mut result| {
                result.offset(offsets[0].0, offsets[0].1, offsets[1].0, offsets[1].1);
                result
            });
            let _ = tx.send(CompareResult {
                left: left_id,
                right: right_id,
                left_rev,
                right_rev,
                generation,
                options: options_key,
                result,
            });
        });
        self.compare_rx = Some(rx);
        self.compare_due = None;
        self.note("Comparing in background...");
        Ok(())
    }

    fn apply_compare(&mut self, result: CompareResult) -> Result<()> {
        if !self.comparing || result.generation != self.compare_generation {
            return Ok(());
        }
        let Some(right) = self.secondary else {
            return Ok(());
        };
        let left = self.primary;
        if self.documents[left].snapshot.id != result.left
            || self.documents[right].snapshot.id != result.right
            || self.documents[left].revision != result.left_rev
            || self.documents[right].revision != result.right_rev
            || self.compare_pair != Some([result.left, result.right])
            || result.options != format!("{:?}", self.compare_options)
        {
            if self.comparing {
                self.compare_due = Some(Instant::now() + Duration::from_millis(300));
            }
            return Ok(());
        }
        self.clear_compare_marks();
        let comparison = result.result?;
        self.differences = comparison.differences;
        for diff in &self.differences {
            for line in diff.left.clone() {
                self.editors[0].send(SCI_MARKERADD, line, 20);
            }
            for line in diff.right.clone() {
                self.editors[1].send(SCI_MARKERADD, line, 21);
            }
            for range in &diff.left_inline {
                self.editors[0].indicator(20, range.clone())?;
            }
            for range in &diff.right_inline {
                self.editors[1].indicator(21, range.clone())?;
            }
        }
        for moved in &comparison.moves {
            self.editors[0].send(SCI_MARKERADD, moved.left, 22);
            self.editors[1].send(SCI_MARKERADD, moved.right, 22);
        }
        if self.compare_options.align {
            let zoom = self.editor().send(SCI_GETZOOM, 0, 0);
            for editor in &self.editors {
                editor.send(SCI_SETZOOM, zoom as usize, 0);
            }
            self.compare_leading = [
                self.editors[0].apply_compare_padding(&comparison.left_padding)?,
                self.editors[1].apply_compare_padding(&comparison.right_padding)?,
            ];
            self.compare_aligned = true;
            self.align_compare_row(self.compare_row);
        }
        let ignored: Vec<_> = [
            (self.compare_options.ignore_whitespace, "whitespace"),
            (self.compare_options.ignore_case, "case"),
            (self.compare_options.ignore_empty_lines, "empty lines"),
            (
                !self.compare_options.ignore_regex.is_empty(),
                "regex matches",
            ),
        ]
        .into_iter()
        .filter_map(|(enabled, label)| enabled.then_some(label))
        .collect();
        self.note(format!(
            "{} difference groups, {} moved lines. Red: left; green: right; purple: moved. {}{}F7 to navigate.",
            self.differences.len(),
            comparison.moves.len(),
            if self.compare_selection.is_some() {
                "Selected lines only. "
            } else {
                ""
            },
            if ignored.is_empty() {
                String::new()
            } else {
                format!("Ignoring {}. ", ignored.join(", "))
            },
        ));
        if self.compare_jump && !self.differences.is_empty() {
            self.difference = self.differences.len() - 1;
            self.navigate_difference(false);
        }
        self.compare_jump = false;
        self.difference = self
            .difference
            .min(self.differences.len().saturating_sub(1));
        Ok(())
    }

    fn navigate_difference(&mut self, previous: bool) {
        if self.differences.is_empty() {
            return;
        }
        let count = self.differences.len();
        self.difference = (self.difference + if previous { count - 1 } else { 1 }) % count;
        let diff = &self.differences[self.difference];
        let mut aligned_row = usize::MAX;
        for (pane, (editor, line)) in [
            (&self.editors[0], diff.left.start),
            (&self.editors[1], diff.right.start),
        ]
        .into_iter()
        .enumerate()
        {
            let line = line.min(editor.send(SCI_GETLINECOUNT, 0, 0).saturating_sub(1) as usize);
            editor.send(SCI_GOTOLINE, line, 0);
            editor.send(SCI_ENSUREVISIBLEENFORCEPOLICY, line, 0);
            let visible = editor.send(SCI_VISIBLEFROMDOCLINE, line, 0).max(0) as usize;
            aligned_row = aligned_row.min(visible + self.compare_leading[pane]);
            editor.send(SCI_SETFIRSTVISIBLELINE, visible.saturating_sub(4), 0);
        }
        if self.compare_aligned {
            self.align_compare_row(aligned_row.saturating_sub(4));
        }
        self.note(format!("Difference {} of {count}", self.difference + 1));
    }

    fn schedule_json(&mut self) {
        self.json_document = None;
        self.json_due = Some(Instant::now() + Duration::from_millis(350));
        self.tree.set_sensitive(false);
    }

    fn launch_json(&mut self) -> Result<()> {
        if self.json_rx.is_some() {
            return Ok(());
        }
        let doc = &self.documents[self.index()];
        let (document, revision) = (doc.snapshot.id, doc.revision);
        let text = self.tool_text(&self.editor())?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(JsonResult {
                document,
                revision,
                result: core::json_tree(&text),
            });
        });
        self.json_rx = Some(rx);
        self.json_due = None;
        self.note("Updating JSON tree...");
        Ok(())
    }

    fn apply_json(&mut self, result: JsonResult) -> Result<()> {
        let doc = &self.documents[self.index()];
        if !self.tree_visible {
            return Ok(());
        }
        if result.document != doc.snapshot.id || result.revision != doc.revision {
            self.schedule_json();
            return Ok(());
        }
        let nodes = match result.result {
            Ok(nodes) => nodes,
            Err(error) => {
                self.json_document = None;
                self.note(format!("JSON paused: {error}"));
                return Ok(());
            }
        };
        let mut expanded = HashSet::new();
        let selected = self
            .tree
            .selection()
            .selected()
            .and_then(|(model, iter)| model.value(&iter, 1).get::<u32>().ok())
            .and_then(|index| self.json_nodes.get(index as usize))
            .map(|node| node.pointer.clone());
        for (node, iter) in self.json_nodes.iter().zip(&self.json_handles) {
            if let Some(path) = self.tree_store.path(iter)
                && self.tree.row_expanded(&path)
            {
                expanded.insert(node.pointer.clone());
            }
        }
        TREE_UPDATING.with(|flag| flag.set(true));
        self.tree_store.clear();
        let mut handles = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            let iter = self
                .tree_store
                .append(node.parent.map(|parent| &handles[parent]));
            self.tree_store
                .set(&iter, &[(0, &node.label), (1, &(index as u32))]);
            handles.push(iter);
        }
        for (node, iter) in nodes.iter().zip(&handles) {
            if let Some(path) = self.tree_store.path(iter)
                && (node.parent.is_none() || expanded.contains(&node.pointer))
            {
                self.tree.expand_row(&path, false);
            }
            if selected.as_ref() == Some(&node.pointer) {
                self.tree.selection().select_iter(iter);
            }
        }
        TREE_UPDATING.with(|flag| flag.set(false));
        self.json_nodes = nodes;
        self.json_handles = handles;
        self.json_document = Some(result.document);
        self.tree.set_sensitive(true);
        self.note("Live JSON tree: select a node to jump to its value.");
        Ok(())
    }

    fn definition_text(path: &Path) -> Result<String> {
        let bytes = session::read_bounded(path, 1024 * 1024)?;
        let (text, encoding) = core::decode(&bytes, None)?;
        if matches!(encoding, Encoding::Legacy(_)) {
            return Err("Definitions must be UTF-8 or BOM-marked Unicode.".into());
        }
        Ok(text)
    }

    fn import_language(&mut self, path: &Path) -> Result<()> {
        let definitions = udl::import(&Self::definition_text(path)?)?;
        let mut languages = self.languages.clone();
        let mut changed = Vec::new();
        for definition in definitions {
            changed.push(languages::add_custom(&mut languages, definition)?);
        }
        self.languages = languages;
        for doc in &mut self.documents {
            if doc.language == 0
                && let Some(path) = &doc.snapshot.path
            {
                doc.language = languages::detect(path, &self.languages);
                doc.snapshot.language = self.languages[doc.language].name.clone();
            }
            if changed.contains(&doc.language) {
                doc.snapshot.language = self.languages[doc.language].name.clone();
                doc.revision += 1;
                doc.styled_revision = None;
            }
        }
        self.make_menu();
        if !self.documents.is_empty() {
            self.refresh_views()?;
        }
        self.touch();
        self.note(format!(
            "Imported {} data-only language definition(s).",
            changed.len()
        ));
        Ok(())
    }

    fn import_api(&mut self, path: &Path) -> Result<()> {
        let language = &self.languages[self.documents[self.index()].language].name;
        let entries = completion::import_api(&Self::definition_text(path)?, language)?;
        let mut api = self.completion_api.clone();
        api.retain(|old| {
            !entries.iter().any(|new| {
                new.language == old.language && new.receiver == old.receiver && new.name == old.name
            })
        });
        api.extend(entries);
        if api.len() > 10_000 {
            return Err("The completion catalog is limited to 10,000 imported overloads.".into());
        }
        self.completion_api = api;
        self.touch();
        self.note("Completion signatures imported for the active language.");
        Ok(())
    }

    fn remove_language(&mut self) -> Result<()> {
        let language = self.documents[self.index()].language;
        if self.languages[language].custom.is_none() {
            return Err("Select a user-defined language first.".into());
        }
        let removed = self.languages.remove(language);
        self.completion_api
            .retain(|api| api.language != removed.name);
        for doc in &mut self.documents {
            if doc.language == language {
                doc.language = 0;
                doc.snapshot.language = self.languages[0].name.clone();
                doc.revision += 1;
                doc.styled_revision = None;
            } else if doc.language > language {
                doc.language -= 1;
            }
        }
        self.make_menu();
        self.refresh_views()?;
        self.touch();
        self.note("User-defined language removed; document text is unchanged.");
        Ok(())
    }

    fn launch_highlight(&mut self) -> Result<()> {
        if self.highlight_rx.is_some() {
            return Ok(());
        }
        let index = [Some(self.primary), self.secondary]
            .into_iter()
            .flatten()
            .find(|index| {
                let doc = &self.documents[*index];
                self.languages[doc.language].uses_container()
                    && doc.styled_revision != Some(doc.revision)
                    && doc.last_edit.elapsed() >= Duration::from_millis(200)
            });
        let Some(index) = index else {
            return Ok(());
        };
        let doc = &self.documents[index];
        let definition = self.languages[doc.language].clone();
        let (document, revision, language) =
            (doc.snapshot.id, doc.revision, definition.name.clone());
        self.scratch.attach(&doc.handle);
        let text = self.scratch.text()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(HighlightResult {
                document,
                revision,
                language,
                result: definition.highlight(&text),
            });
        });
        self.highlight_rx = Some(rx);
        Ok(())
    }

    fn line_operation(&mut self, operation: LineOp) -> Result<()> {
        let editor = self.editor();
        let selection = editor.selection();
        let range = if selection.is_empty() {
            0..editor.length()
        } else {
            let first = editor.send(SCI_LINEFROMPOSITION, selection.start, 0) as usize;
            let last_position = editor.send(SCI_POSITIONBEFORE, selection.end, 0) as usize;
            let last = editor.send(SCI_LINEFROMPOSITION, last_position, 0) as usize;
            let start = editor.send(SCI_POSITIONFROMLINE, first, 0) as usize;
            let end = editor.send(SCI_POSITIONFROMLINE, last + 1, 0);
            start..if end < 0 {
                editor.length()
            } else {
                end as usize
            }
        };
        if range.len() > core::MAX_TOOL_BYTES {
            return Err("Line operations are limited to 16 MiB.".into());
        }
        let out = core::lines(
            &editor.range(range.clone())?,
            operation,
            self.documents[self.index()].snapshot.eol,
        )?;
        editor.replace(range.clone(), &out)?;
        editor.select(range.start..range.start + out.len());
        Ok(())
    }

    fn command(&mut self, command: usize) -> Result<()> {
        let result_focus = self.results.has_focus();
        if result_focus
            && matches!(
                command,
                UNDO | REDO
                    | CUT
                    | PASTE
                    | DUPLICATE
                    | DELETE_LINE
                    | UPPER
                    | LOWER
                    | TITLE_CASE
                    | SENTENCE_CASE
                    | INVERT_CASE
                    | SORT
                    | SORT_DESC
                    | UNIQUE
                    | TRIM
                    | REMOVE_EMPTY
                    | SORT_IGNORE_CASE
                    | SORT_DESC_IGNORE_CASE
                    | SORT_NATURAL
                    | SORT_NUMERIC
                    | SORT_NUMERIC_DESC
                    | SORT_NUMERIC_COMMA
                    | REVERSE_LINES
                    | UNIQUE_ADJACENT
                    | TRIM_START
                    | TRIM_BOTH
                    | REMOVE_EMPTY_ONLY
                    | JOIN_LINES
                    | ADD_NEXT
                    | SELECT_MATCHES
            )
        {
            self.note("Search results are read-only. Focus an editing pane to modify a document.");
            return Ok(());
        }
        let editor = if result_focus && matches!(command, COPY | SELECT_ALL) {
            self.results.editor.clone()
        } else {
            self.editor()
        };
        match command {
            NEW => self.new_document()?,
            OPEN => {
                if let Some(path) = self.dialog(false)? {
                    self.open_path(&path, None)?;
                }
            }
            SAVE | SAVE_AS => {
                self.save_document(command == SAVE_AS)?;
            }
            CLOSE => self.close_document()?,
            EXIT => self.close()?,
            UNDO | REDO | CUT | COPY | PASTE | SELECT_ALL | DUPLICATE | DELETE_LINE => {
                editor.send(
                    match command {
                        UNDO => SCI_UNDO,
                        REDO => SCI_REDO,
                        CUT => SCI_CUT,
                        COPY => SCI_COPY,
                        PASTE => SCI_PASTE,
                        SELECT_ALL => SCI_SELECTALL,
                        DUPLICATE => SCI_SELECTIONDUPLICATE,
                        _ => SCI_LINEDELETE,
                    },
                    0,
                    0,
                );
            }
            UPPER | LOWER | TITLE_CASE | SENTENCE_CASE | INVERT_CASE => {
                let operation = match command {
                    UPPER => CaseOp::Upper,
                    LOWER => CaseOp::Lower,
                    TITLE_CASE => CaseOp::Title,
                    SENTENCE_CASE => CaseOp::Sentence,
                    _ => CaseOp::Invert,
                };
                editor.transform_selections(|text| core::change_case(text, operation))?;
            }
            SORT
            | SORT_DESC
            | UNIQUE
            | TRIM
            | REMOVE_EMPTY
            | SORT_IGNORE_CASE
            | SORT_DESC_IGNORE_CASE
            | SORT_NATURAL
            | SORT_NUMERIC
            | SORT_NUMERIC_DESC
            | SORT_NUMERIC_COMMA
            | REVERSE_LINES
            | UNIQUE_ADJACENT
            | TRIM_START
            | TRIM_BOTH
            | REMOVE_EMPTY_ONLY
            | JOIN_LINES => self.line_operation(match command {
                SORT => LineOp::Sort,
                SORT_DESC => LineOp::SortDescending,
                UNIQUE => LineOp::Unique,
                TRIM => LineOp::Trim,
                REMOVE_EMPTY => LineOp::RemoveEmpty,
                SORT_IGNORE_CASE => LineOp::SortIgnoreCase,
                SORT_DESC_IGNORE_CASE => LineOp::SortDescendingIgnoreCase,
                SORT_NATURAL => LineOp::SortNatural,
                SORT_NUMERIC => LineOp::SortNumeric,
                SORT_NUMERIC_DESC => LineOp::SortNumericDescending,
                SORT_NUMERIC_COMMA => LineOp::SortNumericComma,
                REVERSE_LINES => LineOp::Reverse,
                UNIQUE_ADJACENT => LineOp::UniqueAdjacent,
                TRIM_START => LineOp::TrimStart,
                TRIM_BOTH => LineOp::TrimBoth,
                REMOVE_EMPTY_ONLY => LineOp::RemoveEmptyOnly,
                _ => LineOp::Join,
            })?,
            PARAMETER_HINT => editor.call_tip(
                &self.languages[self.documents[self.index()].language],
                &self.completion_api,
            )?,
            COMPLETE => editor.complete(
                &self.languages[self.documents[self.index()].language],
                &self.completion_api,
                true,
            )?,
            IMPORT_LANGUAGE | IMPORT_API => {
                if let Some(path) = self.dialog(false)? {
                    if command == IMPORT_LANGUAGE {
                        self.import_language(&path)?;
                    } else {
                        self.import_api(&path)?;
                    }
                }
            }
            REMOVE_LANGUAGE => self.remove_language()?,
            ADD_NEXT => {
                editor.send(SCI_MULTIPLESELECTADDNEXT, 0, 0);
            }
            SELECT_MATCHES => {
                editor.send(SCI_MULTIPLESELECTADDEACH, 0, 0);
            }
            FIND => self.show_search()?,
            FIND_FILES => {
                self.search.folder_visible = true;
                if self.search.directory.text().is_empty() {
                    let directory = self.documents[self.index()]
                        .snapshot
                        .path
                        .as_ref()
                        .and_then(|path| path.parent().map(Path::to_path_buf))
                        .or_else(|| std::env::current_dir().ok());
                    if let Some(directory) = directory {
                        self.search.directory.set_text(&directory.to_string_lossy());
                    }
                }
                self.show_search()?;
            }
            FIND_FILES_RUN => self.start_find_files()?,
            FIND_FOLDER => self.choose_search_folder(),
            TAB_PIN => self.pin_tab(self.documents[self.index()].snapshot.id),
            TAB_LEFT | TAB_RIGHT => {
                self.event(Event::StepTab(
                    self.documents[self.index()].snapshot.id,
                    command == TAB_LEFT,
                ))?;
            }
            SEARCH_CLOSE => {
                self.search.visible = false;
                self.layout();
                editor.focus();
            }
            FIND_NEXT | FIND_PREVIOUS => self.find(command == FIND_PREVIOUS)?,
            FIND_ALL_CURRENT => self.start_find_all(false)?,
            FIND_ALL_OPEN => self.start_find_all(true)?,
            RESULTS_TOGGLE => {
                self.set_results_visible(!self.results.visible);
                if self.results.visible {
                    self.results.editor.focus();
                } else {
                    self.editor().focus();
                }
            }
            RESULTS_NEXT => self.next_result(false)?,
            RESULTS_PREVIOUS => self.next_result(true)?,
            RESULTS_ACTIVATE => self.activate_result_position(self.results.editor.position())?,
            RESULTS_CLEAR => self.clear_find_results()?,
            RESULTS_CLOSE => {
                self.set_results_visible(false);
                self.editor().focus();
            }
            RESULTS_CANCEL => {
                if let Some(job) = &self.results.job {
                    job.cancelled.store(true, Ordering::Relaxed);
                    self.result_message("Cancelling search...");
                }
            }
            REPLACE | REPLACE_ALL => self.replace(command == REPLACE_ALL)?,
            SPLIT => {
                self.clear_compare();
                self.secondary = if self.secondary.is_some() {
                    None
                } else {
                    Some(self.primary)
                };
                if self.secondary.is_none() {
                    self.focused = 0;
                }
                self.refresh_views()?;
                self.update_tabs();
                self.note("Click a pane, then a tab, to choose its document. F6 switches panes.");
            }
            MAP => {
                self.map_visible = !self.map_visible;
                self.configure_map();
                self.layout();
            }
            WRAP => {
                self.wrap = !self.wrap;
                for editor in &self.editors {
                    editor.send(SCI_SETWRAPMODE, self.effective_wrap() as usize, 0);
                }
            }
            MONITOR_FILES => {
                self.monitor_files = !self.monitor_files;
                if self.monitor_files {
                    self.monitor.reset();
                }
                self.last_monitor = Instant::now() - Duration::from_secs(1);
                if let Some(item) = &self.monitor_item {
                    SYMBOLS_UPDATING.with(|flag| flag.set(true));
                    item.set_active(self.monitor_files);
                    SYMBOLS_UPDATING.with(|flag| flag.set(false));
                }
                self.touch();
                self.note(if self.monitor_files {
                    "External file monitoring enabled."
                } else {
                    "External file monitoring disabled."
                });
            }
            SYMBOL_SPACE | SYMBOL_EOL | SYMBOL_NONPRINTING | SYMBOL_CONTROLS | SYMBOL_ALL
            | SYMBOL_INDENT | SYMBOL_WRAP => {
                match command {
                    SYMBOL_SPACE => self.show_symbols.whitespace = !self.show_symbols.whitespace,
                    SYMBOL_EOL => self.show_symbols.eol = !self.show_symbols.eol,
                    SYMBOL_NONPRINTING => {
                        self.show_symbols.non_printing = !self.show_symbols.non_printing
                    }
                    SYMBOL_CONTROLS => self.show_symbols.controls = !self.show_symbols.controls,
                    SYMBOL_ALL => self.show_symbols.toggle_all(),
                    SYMBOL_INDENT => {
                        self.show_symbols.indent_guides = !self.show_symbols.indent_guides
                    }
                    SYMBOL_WRAP => self.show_symbols.wrap_markers = !self.show_symbols.wrap_markers,
                    _ => unreachable!(),
                }
                self.apply_symbols();
                self.touch();
            }
            ZOOM_RESET => {
                for editor in &self.editors {
                    editor.send(SCI_SETZOOM, 0, 0);
                }
            }
            EDITOR_FONT => {
                if let Some(font) = self.choose_editor_font()? {
                    self.set_editor_font(font);
                    self.editor().focus();
                }
            }
            THEME_SYSTEM | THEME_LIGHT | THEME_DARK => {
                self.theme = match command {
                    THEME_LIGHT => "light",
                    THEME_DARK => "dark",
                    _ => "system",
                }
                .into();
                self.apply_theme();
                self.touch();
            }
            COMPARE => self.start_compare()?,
            COMPARE_SELECTION => self.compare_selected_lines()?,
            COMPARE_CLIPBOARD => self.compare_snapshot(true)?,
            COMPARE_SAVED => self.compare_snapshot(false)?,
            COMPARE_SETTINGS => self.choose_compare_options()?,
            COMPARE_CLEAR => {
                self.clear_compare();
                self.note("Compare cleared.");
            }
            DIFF_NEXT | DIFF_PREVIOUS => self.navigate_difference(command == DIFF_PREVIOUS),
            JSON_FORMAT | JSON_COMPACT => {
                let text = core::format_json(
                    &self.tool_text(&editor)?,
                    command == JSON_COMPACT,
                    self.documents[self.index()].snapshot.eol,
                )?;
                editor.replace_all(&text)?;
            }
            JSON_TREE | JSON_REFRESH => {
                self.tree_visible = command == JSON_REFRESH || !self.tree_visible;
                if self.tree_visible {
                    self.schedule_json();
                    self.json_due = Some(Instant::now());
                    self.launch_json()?;
                } else {
                    self.json_due = None;
                    self.json_rx = None;
                }
                self.layout();
            }
            EOL_CRLF | EOL_LF | EOL_CR => {
                let eol = match command {
                    EOL_LF => Eol::Lf,
                    EOL_CR => Eol::Cr,
                    _ => Eol::CrLf,
                };
                editor.send(SCI_CONVERTEOLS, eol.scintilla(), 0);
                editor.send(SCI_SETEOLMODE, eol.scintilla(), 0);
                let index = self.index();
                self.documents[index].snapshot.eol = eol;
                self.documents[index].metadata_dirty = true;
                self.documents[index].snapshot.dirty = true;
                self.touch();
                self.update_tabs();
                self.update_status();
            }
            ABOUT => {
                message(
                    Some(&self.window),
                    concat!(
                        "rstpd ",
                        env!("CARGO_PKG_VERSION"),
                        "\nNative GTK3 application with statically linked Scintilla + Lexilla.\nStock Adwaita; no plugins, script host or updater.\n\nAlt+drag: rectangular selection\nCtrl+click: multiple carets\nCtrl+D / Ctrl+Shift+L: next / all occurrences\nF6: other pane; Ctrl+Tab: next tab\nCtrl+Space: completion; Ctrl+Shift+Space: function parameters\nCtrl+mouse wheel: zoom\nCtrl+Alt+J: format JSON/JSON5; Ctrl+Alt+T: JSON tree\nF7 / Shift+F7: next / previous difference\n\nCompare, JSON and Markdown refresh after edits.\nLanguage menu: import data-only language/API XML.\nClose the app to preserve all tabs, including unsaved text.\nRecovery is local plaintext in $XDG_STATE_HOME/rstpd-gtk\n(or ~/.local/state/rstpd-gtk), unless --session-dir is supplied.\nFiles: 128 MiB; search, line and JSON tools: 16 MiB.\n\nIndependent application; not affiliated with Notepad++."
                    ),
                    gtk::MessageType::Info,
                    &[("_Close", gtk::ResponseType::Close)],
                );
            }
            id if (ENCODING_BASE..ENCODING_BASE + core::encoding_options().len()).contains(&id) => {
                let encoding = core::encoding_options()[id - ENCODING_BASE].clone();
                encoding.encode(&editor.text()?)?;
                let index = self.index();
                self.documents[index].snapshot.encoding = encoding;
                self.documents[index].metadata_dirty = true;
                self.documents[index].snapshot.dirty = true;
                self.touch();
                self.update_tabs();
                self.note("Encoding will be applied on Save.");
            }
            id if (REOPEN_BASE..REOPEN_BASE + core::encoding_options().len()).contains(&id) => {
                let index = self.index();
                let path = self.documents[index]
                    .snapshot
                    .path
                    .clone()
                    .ok_or("This document has no file to reopen.")?;
                if confirm(
                    &self.window,
                    "Reload the file using this encoding? Current edits will be discarded.",
                ) {
                    let bytes = session::read_bounded(&path, core::MAX_DOCUMENT_BYTES)?;
                    let (text, encoding) =
                        core::decode(&bytes, Some(&core::encoding_options()[id - REOPEN_BASE]))?;
                    editor.set_text(&text)?;
                    let doc = &mut self.documents[index];
                    doc.snapshot.encoding = encoding;
                    doc.snapshot.eol = Eol::detect(&text);
                    doc.snapshot.disk_hash = Some(session::fingerprint(&bytes));
                    doc.snapshot.dirty = false;
                    doc.base_dirty = false;
                    doc.metadata_dirty = false;
                    doc.revision += 1;
                    doc.styled_revision = None;
                    self.invalidate_tools(self.documents[index].snapshot.id);
                    self.touch();
                    self.refresh_views()?;
                    self.update_tabs();
                }
            }
            id if (LANGUAGE_BASE..LANGUAGE_BASE + self.languages.len()).contains(&id) => {
                let index = self.index();
                self.documents[index].language = id - LANGUAGE_BASE;
                self.documents[index].revision += 1;
                self.documents[index].styled_revision = None;
                self.documents[index].snapshot.language =
                    self.languages[id - LANGUAGE_BASE].name.clone();
                self.refresh_views()?;
                self.touch();
                self.update_status();
            }
            _ => return Err(format!("Unknown command: {command}")),
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<()> {
        self.poll_monitor();
        self.poll_find_results()?;
        while let Ok((revision, result)) = self.recovery.rx.try_recv() {
            self.recovery_busy = false;
            match result {
                Ok(()) => {
                    self.recovered_revision = revision;
                    self.recovery_error = None;
                }
                Err(error) => {
                    self.recovery_error = Some(error);
                }
            }
            self.update_status();
        }
        if let Some(rx) = &self.compare_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.compare_rx = None;
                    if let Err(error) = self.apply_compare(result) {
                        self.note(format!("Compare paused: {error}"));
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.compare_rx = None;
                    return Err("Compare worker stopped unexpectedly.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.comparing
            && self.compare_rx.is_none()
            && self.compare_due.is_some_and(|due| Instant::now() >= due)
        {
            self.compare_due = None;
            if let Err(error) = self.launch_compare() {
                self.note(format!("Compare paused: {error}"));
            }
        }
        if let Some(rx) = &self.json_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.json_rx = None;
                    self.apply_json(result)?;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.json_rx = None;
                    return Err("JSON worker stopped unexpectedly.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.tree_visible
            && self.json_rx.is_none()
            && self.json_due.is_some_and(|due| Instant::now() >= due)
        {
            self.json_due = None;
            if let Err(error) = self.launch_json() {
                self.note(format!("JSON paused: {error}"));
            }
        }
        if let Some(rx) = &self.highlight_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.highlight_rx = None;
                    if let Some(index) = self.documents.iter().position(|doc| {
                        doc.snapshot.id == result.document
                            && doc.revision == result.revision
                            && self.languages[doc.language].uses_container()
                            && self.languages[doc.language].name == result.language
                    }) {
                        self.documents[index].styled_revision = Some(result.revision);
                        self.scratch.attach(&self.documents[index].handle);
                        match result.result {
                            Ok(highlight) => self.scratch.highlight(&highlight)?,
                            Err(error) => {
                                self.scratch.clear_styles();
                                self.note(format!("Highlighting paused: {error}"));
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.highlight_rx = None;
                    return Err("Highlighting worker stopped unexpectedly.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.launch_highlight()?;
        if self.revision != self.recovered_revision
            && !self.recovery_busy
            && self.last_autosave.elapsed() >= Duration::from_secs(3)
        {
            self.recovery.submit(self.revision, self.snapshot()?)?;
            self.recovery_busy = true;
            self.last_autosave = Instant::now();
            self.update_status();
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        // The flush blocks on a full recovery write; leave the reason on the status bar
        // so the dialog below (which pumps the main loop) renders it if anything fails.
        self.note("Saving recovery before exit...");
        let saved = self
            .snapshot()
            .and_then(|snapshot| self.recovery.flush(self.revision + 1, snapshot));
        if let Err(error) = saved {
            self.recovery_error = Some(error.clone());
            self.update_status();
            let unsaved = self
                .documents
                .iter()
                .filter(|doc| doc.snapshot.dirty)
                .count();
            if message(
                Some(&self.window),
                &format!(
                    "Recovery could not be saved: {error}\n\nQuit anyway? {unsaved} tab(s) with unsaved edits will be lost.\nChoose Cancel to stay open and save them with File > Save as."
                ),
                gtk::MessageType::Error,
                &[
                    ("_Cancel", gtk::ResponseType::Cancel),
                    ("_Quit anyway", gtk::ResponseType::Yes),
                ],
            ) != gtk::ResponseType::Yes
            {
                return Ok(());
            }
        }
        if let Some(job) = &self.results.job {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        self.exiting = true;
        self.window.hide();
        unsafe {
            self.window.destroy();
        }
        if gtk::main_level() > 0 {
            gtk::main_quit();
        }
        Ok(())
    }

    fn event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Command(command) => self.command(command)?,
            Event::Close => self.close()?,
            Event::Error(error) => return Err(error),
            Event::Theme => {
                if self.theme == "system" {
                    self.apply_theme();
                }
            }
            Event::MoveTab(id, requested) => self.move_tab(id, requested)?,
            Event::PinTab(id) => self.pin_tab(id),
            Event::SplitTab(id) => self.split_tab(id)?,
            Event::CompareTabs(current, target) => self.compare_tabs(current, target)?,
            Event::StepTab(id, previous) => {
                if let Some(index) = self.documents.iter().position(|doc| doc.snapshot.id == id) {
                    let requested = if previous {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(self.documents.len() - 1)
                    };
                    self.move_tab(id, requested)?;
                }
            }
            Event::Tab(id) | Event::CloseTab(id) => {
                if let Some(index) = self.documents.iter().position(|d| d.snapshot.id == id) {
                    let close = matches!(event, Event::CloseTab(_));
                    if !close && index == self.index() {
                        return Ok(());
                    }
                    if self.comparing {
                        self.clear_compare();
                    }
                    self.switch(index)?;
                    if close {
                        self.close_document()?;
                    }
                }
            }
            Event::NextTab(previous) => {
                let len = self.documents.len();
                if self.comparing {
                    self.clear_compare();
                }
                self.switch((self.index() + if previous { len - 1 } else { 1 }) % len)?;
            }
            Event::OtherPane => {
                if self.secondary.is_some() {
                    self.focused = 1 - self.focused;
                    self.editor().focus();
                    self.configure_map();
                    self.update_tabs();
                    self.update_status();
                    if self.tree_visible {
                        self.schedule_json();
                    }
                }
            }
            Event::Escape => {
                if self.results.has_focus() {
                    self.command(RESULTS_CLOSE)?;
                    return Ok(());
                }
                self.editor().send(SCI_AUTOCCANCEL, 0, 0);
                self.editor().send(SCI_CALLTIPCANCEL, 0, 0);
                if self.search.visible {
                    self.command(SEARCH_CLOSE)?;
                }
            }
            Event::Focus(pane, id) => {
                if self.pane_ids[pane].get() == id && (pane == 0 || self.secondary.is_some()) {
                    let changed = self.focused != pane;
                    self.focused = pane;
                    if changed {
                        self.configure_map();
                        self.update_tabs();
                        self.update_status();
                        if self.tree_visible {
                            self.schedule_json();
                        }
                    }
                }
            }
            Event::Updated(pane, id) => {
                if self.pane_ids[pane].get() == id {
                    self.editors[pane].update_line_number_margin();
                    if self.compare_aligned {
                        let editor = &self.editors[pane];
                        let visible = editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0);
                        let expected = self.compare_row.saturating_sub(self.compare_leading[pane]);
                        let zoom = editor.send(SCI_GETZOOM, 0, 0);
                        if self.editors[1 - pane].send(SCI_GETZOOM, 0, 0) != zoom {
                            self.editors[1 - pane].send(SCI_SETZOOM, zoom as usize, 0);
                            for frame in &self.pane_frames {
                                frame.queue_resize();
                            }
                        }
                        let row = if visible == expected as isize {
                            self.compare_row
                        } else if visible <= 0 {
                            0
                        } else {
                            visible as usize + self.compare_leading[pane]
                        };
                        self.align_compare_row(row);
                    }
                }
                if pane == self.focused && self.pane_ids[pane].get() == id {
                    let index = self.index();
                    let editor = self.editor();
                    let position = editor.position();
                    if self.documents[index].snapshot.caret != position {
                        self.documents[index].snapshot.caret = position;
                        self.touch();
                        if editor.send(SCI_CALLTIPACTIVE, 0, 0) != 0 {
                            editor.call_tip(
                                &self.languages[self.documents[index].language],
                                &self.completion_api,
                            )?;
                        }
                    }
                    self.update_map_view();
                    if self.comparing && self.secondary.is_some() {
                        let visible = editor.send(SCI_GETFIRSTVISIBLELINE, 0, 0);
                        if !self.compare_options.align {
                            let other = &self.editors[1 - pane];
                            let source = editor
                                .send(SCI_DOCLINEFROMVISIBLE, visible.max(0) as usize, 0)
                                .max(0) as usize;
                            let target =
                                core::corresponding_line(&self.differences, source, pane == 1);
                            let line = other.send(SCI_VISIBLEFROMDOCLINE, target, 0).max(0);
                            if other.send(SCI_GETFIRSTVISIBLELINE, 0, 0) != line {
                                other.send(SCI_SETFIRSTVISIBLELINE, line as usize, 0);
                            }
                        } else if !self.compare_aligned && !self.compare_jump {
                            self.compare_row = self.current_compare_row();
                            self.compare_native_row[pane] = visible.max(0) as usize;
                        }
                    }
                    self.update_status();
                }
            }
            Event::Changed(_pane, id) => {
                if let Some(index) = self.documents.iter().position(|doc| doc.snapshot.id == id) {
                    self.last_zero_match = None;
                    self.scratch.attach(&self.documents[index].handle);
                    if self.scratch.length() > core::MAX_DOCUMENT_BYTES {
                        self.scratch.send(SCI_UNDO, 0, 0);
                        return Err(
                            "The edit was undone because it exceeded the 128 MiB document limit."
                                .into(),
                        );
                    }
                    let doc = &mut self.documents[index];
                    let dirty = doc.base_dirty
                        || doc.metadata_dirty
                        || self.scratch.send(SCI_GETMODIFY, 0, 0) != 0;
                    let changed = doc.snapshot.dirty != dirty;
                    doc.snapshot.dirty = dirty;
                    doc.revision += 1;
                    doc.last_edit = Instant::now();
                    self.invalidate_tools(id);
                    self.touch();
                    if changed {
                        self.update_tabs();
                    }
                    self.update_status();
                }
            }
            Event::Style(pane, id) => {
                if self.pane_ids[pane].get() == id
                    && let Some(index) = self.documents.iter().position(|doc| doc.snapshot.id == id)
                    && self.languages[self.documents[index].language].uses_container()
                    && self.editors[pane].send(SCI_GETENDSTYLED, 0, 0)
                        < self.editors[pane].length() as isize
                {
                    self.documents[index].styled_revision = None;
                }
            }
            Event::Character(pane, id, ch) => {
                if pane == self.focused && self.pane_ids[pane].get() == id {
                    let editor = self.editor();
                    let language = &self.languages[self.documents[self.index()].language];
                    if char::from_u32(ch as u32).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        editor.complete(language, &self.completion_api, false)?;
                    }
                    if ch == b'.' as i32 {
                        editor.complete(language, &self.completion_api, true)?;
                    }
                    editor.call_tip(language, &self.completion_api)?;
                    if ch == b'\n' as i32 && editor.send(SCI_GETSELECTIONS, 0, 0) == 1 {
                        let line = editor.send(SCI_LINEFROMPOSITION, editor.position(), 0) as usize;
                        if line > 0 {
                            editor.send(
                                SCI_SETLINEINDENTATION,
                                line,
                                editor.send(SCI_GETLINEINDENTATION, line - 1, 0),
                            );
                            let pos = editor.send(SCI_GETLINEINDENTPOSITION, line, 0) as usize;
                            editor.select(pos..pos);
                        }
                    }
                }
            }
            Event::Tree(index) => {
                if self.json_document != Some(self.documents[self.index()].snapshot.id) {
                    self.note("JSON tree is updating; navigation resumes when parsing completes.");
                } else if let Some(node) = self.json_nodes.get(index) {
                    self.editor().select(node.span.clone());
                    self.note(format!(
                        "JSON pointer: {}",
                        if node.pointer.is_empty() {
                            "/"
                        } else {
                            &node.pointer
                        }
                    ));
                }
            }
            Event::Uris(text) => {
                let mut failures = Vec::new();
                for uri in text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                {
                    let file = gio::File::for_uri(uri);
                    match file.path() {
                        Some(path) => {
                            if let Err(error) = self.open_path(&path, None) {
                                failures.push(error);
                            }
                        }
                        None => {
                            failures.push(format!("Only local file drops are supported: {uri}"))
                        }
                    }
                }
                if let Some(first) = failures.first() {
                    return Err(match failures.len() {
                        1 => first.clone(),
                        count => format!("{first} ({} more file(s) also failed.)", count - 1),
                    });
                }
            }
            Event::Map(y) => {
                let first = self.map.send(SCI_GETFIRSTVISIBLELINE, 0, 0);
                let height = self.map.send(SCI_TEXTHEIGHT, 0, 0).max(1);
                let line = (first + y.max(0) as isize / height)
                    .min(self.editor().send(SCI_GETLINECOUNT, 0, 0) - 1)
                    .max(0);
                self.editor().send(SCI_GOTOLINE, line as usize, 0);
                self.editor().focus();
            }
            Event::MapScroll(delta) => {
                self.map.send(SCI_LINESCROLL, 0, delta);
            }
            Event::CompareScroll(pane, id, delta) => {
                if self.compare_aligned && self.pane_ids[pane].get() == id {
                    self.align_compare_row(self.compare_row.saturating_add_signed(delta));
                }
            }
            Event::ResultActivate(position) => self.activate_result_position(position)?,
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            self.window.destroy();
        }
    }
}

#[cfg(test)]
mod tests;

fn keyboard(event: &gdk::EventKey) -> glib::Propagation {
    use gdk::keys::constants as key;
    let state = event.state();
    let control = state.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    let alt = state.contains(gdk::ModifierType::MOD1_MASK);
    let value = event.keyval().to_lower();
    let command = match (control, shift, alt, value) {
        (true, false, false, key::n) => Some(NEW),
        (true, false, false, key::o) => Some(OPEN),
        (true, false, false, key::s) => Some(SAVE),
        (true, true, false, key::s) => Some(SAVE_AS),
        (true, false, false, key::w) => Some(CLOSE),
        (true, false, false, key::f | key::h) => Some(FIND),
        (true, true, false, key::f) => Some(FIND_FILES),
        (true, true, false, key::Page_Up) => Some(TAB_LEFT),
        (true, true, false, key::Page_Down) => Some(TAB_RIGHT),
        (true, false, false, key::d) => Some(ADD_NEXT),
        (true, true, false, key::l) => Some(SELECT_MATCHES),
        (true, true, false, key::u) => Some(UPPER),
        (true, false, false, key::u) => Some(LOWER),
        (true, false, true, key::j) => Some(JSON_FORMAT),
        (true, false, true, key::t) => Some(JSON_TREE),
        (true, false, true, key::Right) => Some(SPLIT),
        (false, false, false, key::F3) => Some(FIND_NEXT),
        (false, true, false, key::F3) => Some(FIND_PREVIOUS),
        (false, false, false, key::F4) => Some(RESULTS_NEXT),
        (false, true, false, key::F4) => Some(RESULTS_PREVIOUS),
        (true, false, true, key::Return | key::KP_Enter) => Some(FIND_ALL_CURRENT),
        (true, true, false, key::Return | key::KP_Enter) => Some(FIND_ALL_OPEN),
        (true, false, true, key::r) => Some(RESULTS_TOGGLE),
        (false, false, false, key::F7) => Some(DIFF_NEXT),
        (false, true, false, key::F7) => Some(DIFF_PREVIOUS),
        (true, false, false, key::space) => Some(COMPLETE),
        (true, true, false, key::space) => Some(PARAMETER_HINT),
        _ => None,
    };
    let event = if let Some(command) = command {
        Event::Command(command)
    } else if control && !alt && matches!(value, key::Tab | key::ISO_Left_Tab) {
        Event::NextTab(shift)
    } else if !control && !alt && value == key::F6 {
        Event::OtherPane
    } else if !control && !alt && value == key::Escape {
        Event::Escape
    } else {
        return glib::Propagation::Proceed;
    };
    queue(event);
    glib::Propagation::Stop
}

pub fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (mut paths, mut language_paths, mut api_paths) = (Vec::new(), Vec::new(), Vec::new());
    let mut session_dir = None;
    let mut positional = false;
    while let Some(arg) = args.next() {
        if positional {
            paths.push(PathBuf::from(arg));
        } else if arg == "--" {
            positional = true;
        } else if arg == "--help" || arg == "-h" {
            println!(
                "rstpd {}\nUsage: rstpd [--session-dir DIR] [--import-language XML] [--completion-api XML] [--] [FILES...]\nNative GTK3/Adwaita editor. Close the app to preserve unsaved tabs.",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        } else if arg == "--version" {
            println!("rstpd {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        } else if arg == "--session-dir" {
            session_dir = Some(PathBuf::from(
                args.next().ok_or("--session-dir requires a directory.")?,
            ));
        } else if arg == "--import-language" {
            language_paths.push(PathBuf::from(
                args.next()
                    .ok_or("--import-language requires an XML file.")?,
            ));
        } else if arg == "--completion-api" {
            api_paths.push(PathBuf::from(
                args.next()
                    .ok_or("--completion-api requires an XML file.")?,
            ));
        } else if arg.to_string_lossy().starts_with('-') {
            return Err(format!(
                "Unknown option: {}. Use -- before filenames beginning with '-'.",
                arg.to_string_lossy()
            ));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    glib::set_prgname(Some("io.github.Sockolet.rstpd-gtk"));
    glib::set_application_name("rstpd");
    gtk::init().map_err(|e| format!("Could not initialize GTK: {e}"))?;
    let directory = match session_dir {
        Some(path) => path,
        None => session::linux_default_directory()?,
    };
    let mut app = App::new(&directory)?;
    for path in language_paths {
        app.import_language(&path)?;
    }
    for path in paths {
        if let Err(error) = app.open_path(&path, None) {
            show_error(&error);
        }
    }
    if app.documents.is_empty() {
        app.new_document()?;
    }
    for path in api_paths {
        app.import_api(&path)?;
    }
    app.window.show_all();
    app.apply_theme();
    app.layout();
    app.editor().focus();
    let window = app.window.clone();
    let app = Rc::new(RefCell::new(app));
    let state = app.clone();
    let mut last_tick = Instant::now();
    let timer = glib::timeout_add_local(Duration::from_millis(16), move || {
        for _ in 0..512 {
            let Some(event) = EVENTS.with(|events| events.borrow_mut().pop_front()) else {
                break;
            };
            let result = state.borrow_mut().event(event);
            if let Err(error) = result {
                eprintln!("rstpd: {error}");
                message(
                    Some(&window),
                    &error,
                    gtk::MessageType::Error,
                    &[("_Close", gtk::ResponseType::Close)],
                );
            }
            if state.borrow().exiting {
                return glib::ControlFlow::Break;
            }
        }
        if last_tick.elapsed() >= Duration::from_millis(250) {
            if let Err(error) = state.borrow_mut().tick() {
                eprintln!("rstpd: {error}");
                message(
                    Some(&window),
                    &error,
                    gtk::MessageType::Error,
                    &[("_Close", gtk::ResponseType::Close)],
                );
            }
            last_tick = Instant::now();
        }
        glib::ControlFlow::Continue
    });
    gtk::main();
    if !app.borrow().exiting {
        timer.remove();
        return Err("The GTK event loop stopped before recovery was saved.".into());
    }
    Ok(())
}
