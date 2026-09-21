use gtk::prelude::*;
use std::rc::Rc;

pub struct Button {
    pub command: usize,
    pub name: &'static str,
    pub tooltip: &'static str,
    pub icon: &'static str,
}

pub fn build(buttons: &[Button], activate: impl Fn(usize) + 'static) -> gtk::Toolbar {
    let toolbar = gtk::Toolbar::new();
    toolbar.set_style(gtk::ToolbarStyle::Icons);
    toolbar.set_icon_size(gtk::IconSize::SmallToolbar);
    let activate = Rc::new(activate);
    for button in buttons {
        let item = gtk::ToolButton::new(None::<&gtk::Widget>, Some(button.name));
        item.set_icon_name(Some(button.icon));
        WidgetExt::set_tooltip_text(&item, Some(button.tooltip));
        let command = button.command;
        let activate = activate.clone();
        item.connect_clicked(move |_| activate(command));
        toolbar.insert(&item, -1);
    }
    toolbar
}
