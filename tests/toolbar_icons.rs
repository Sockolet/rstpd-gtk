use gtk::prelude::*;
use rstpd::toolbar::{self, Button};

#[test]
fn toolbar_uses_native_symbolic_icons_and_accessible_labels() {
    gtk::init().expect("Run GUI tests in a desktop session or with xvfb-run");
    let buttons = [
        Button {
            command: 1,
            name: "New",
            tooltip: "New document (Ctrl+N)",
            icon: "document-new-symbolic",
        },
        Button {
            command: 2,
            name: "Open",
            tooltip: "Open file (Ctrl+O)",
            icon: "document-open-symbolic",
        },
        Button {
            command: 3,
            name: "Save",
            tooltip: "Save document (Ctrl+S)",
            icon: "document-save-symbolic",
        },
    ];
    let called = std::rc::Rc::new(std::cell::Cell::new(0));
    let command = called.clone();
    let toolbar = toolbar::build(&buttons, move |id| command.set(id));
    assert_eq!(toolbar.n_items(), buttons.len() as i32);
    let theme = gtk::IconTheme::default().unwrap();
    for (index, expected) in buttons.iter().enumerate() {
        let item = toolbar
            .nth_item(index as i32)
            .unwrap()
            .downcast::<gtk::ToolButton>()
            .unwrap();
        assert_eq!(item.label().as_deref(), Some(expected.name));
        assert_eq!(item.tooltip_text().as_deref(), Some(expected.tooltip));
        assert_eq!(item.icon_name().as_deref(), Some(expected.icon));
        assert!(theme.has_icon(expected.icon));
        item.emit_by_name::<()>("clicked", &[]);
        assert_eq!(called.get(), expected.command);
    }
}
