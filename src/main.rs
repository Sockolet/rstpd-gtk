fn main() {
    if let Err(error) = rstpd::ui::run() {
        rstpd::ui::show_error(&error);
        std::process::exit(1);
    }
}
