use gtk::prelude::*;
use rstpd::{
    editor::{self, Editor, Palette, sci::*},
    languages,
};
use std::path::Path;

fn color_at(editor: &Editor, text: &str, needle: &str) -> isize {
    let position = text.find(needle).unwrap();
    let style = editor.send(SCI_GETSTYLEAT, position, 0) as usize;
    editor.send(SCI_STYLEGETFORE, style, 0)
}

#[test]
fn markdown_has_visible_syntax_in_both_themes() {
    unsafe {
        gtk::init().expect("GTK3 requires a display; run this test under xvfb-run when headless.");
        let parent = gtk::Window::new(gtk::WindowType::Toplevel);
        parent.set_default_size(900, 700);
        let editor = Editor::new().unwrap();
        parent.add(editor.widget());
        parent.show_all();
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        let document = editor.create_document().unwrap();
        editor.attach(&document);
        let catalog = languages::catalog(&editor::available_lexers());
        let language = &catalog[languages::detect(Path::new("highlighting.md"), &catalog)];
        assert_eq!(language.name, "Markdown");
        let text = include_str!("fixtures/highlighting.md");
        editor.set_text(text).unwrap();
        for dark in [false, true] {
            editor.language(language, Palette::new(dark)).unwrap();
            editor.send(SCI_COLOURISE, 0, -1);
            if language.uses_container() {
                editor
                    .highlight(&language.highlight(text).unwrap())
                    .unwrap();
            }
            assert_ne!(
                editor.send(SCI_GETSTYLEAT, 0, 0),
                editor.send(SCI_GETSTYLEAT, text.find("Plain body").unwrap(), 0),
                "The Markdown lexer should distinguish headings from body text."
            );
            assert_ne!(
                color_at(&editor, text, "# A"),
                color_at(&editor, text, "Plain body"),
                "Recognized Markdown headings must not render in the same color as body text."
            );
            assert_ne!(
                color_at(&editor, text, "visible heading"),
                color_at(&editor, text, "Plain body"),
                "Heading text, not just the # marker, must be highlighted."
            );
            let bold = editor.send(SCI_GETSTYLEAT, text.find("bold words").unwrap(), 0) as usize;
            assert_ne!(editor.send(SCI_STYLEGETBOLD, bold, 0), 0);
            let italic =
                editor.send(SCI_GETSTYLEAT, text.find("italic words").unwrap(), 0) as usize;
            assert_ne!(editor.send(SCI_STYLEGETITALIC, italic, 0), 0);
            assert_ne!(
                color_at(&editor, text, "inline_code"),
                color_at(&editor, text, "Plain body")
            );
            assert_ne!(
                color_at(&editor, text, "fn example"),
                color_at(&editor, text, "Plain body")
            );
            assert_ne!(
                editor.send(
                    SCI_INDICATORVALUEAT,
                    rstpd::markdown::STRIKE_INDICATOR,
                    text.find("removed words").unwrap() as isize
                ),
                0
            );
        }
        for (path, text, needles) in [
            (
                "example.py",
                "class Widget:\n    def render(self):\n        return \"hello\" # comment\n",
                vec!["class", "Widget", "render", "\"hello\"", "# comment"],
            ),
            (
                "example.html",
                "<section title=\"hello\"><?php echo 'embedded'; ?></section>",
                vec!["section", "title", "\"hello\""],
            ),
            (
                "example.css",
                "article { color: red; margin: 10px; }",
                vec!["article", "color", "red"],
            ),
        ] {
            let language = &catalog[languages::detect(Path::new(path), &catalog)];
            editor.set_text(text).unwrap();
            for dark in [false, true] {
                let palette = Palette::new(dark);
                editor.language(language, palette).unwrap();
                editor.send(SCI_COLOURISE, 0, -1);
                let mut colors = std::collections::HashSet::new();
                for needle in &needles {
                    let color = color_at(&editor, text, needle);
                    assert_ne!(
                        color, palette.background as isize,
                        "Invisible token {needle} in {path}"
                    );
                    assert_ne!(
                        color, palette.text as isize,
                        "Unstyled token {needle} in {path}"
                    );
                    colors.insert(color);
                }
                assert!(
                    colors.len() >= 3,
                    "Token categories collapse to the same color in {path}"
                );
            }
        }
        parent.destroy();
    }
}
