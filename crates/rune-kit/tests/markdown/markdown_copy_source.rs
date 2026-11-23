//! `Markdown::copy_source`: copying a selection copies the Markdown source of the top-level
//! blocks it touches, the whole source verbatim for all of them, and the drawn text for part
//! of one block; without it, the drawn text laid out as plain text. And an app's `on_key` sees Cmd+A before a selectable note does.

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};
use rune_kit::{Kit, Markdown};

const SRC: &str = "# Title\n\nHello **world** and [link](https://x.dev/a).\n\n- alpha\n- [x] beta\n\n```rust\nfn main() {}\n```\n\nLast *line*.\n";

#[derive(Default)]
struct S { source: bool }

fn view(s: &S, _: &mut Cx<S>) -> El {
    let k = Kit::neutral(true);
    col().w_full().padding(10.0).child(Markdown::new(SRC).selectable(true).copy_source(s.source).build(&k).key("note"))
}

fn runtime(source: bool) -> (Runtime<S>, rune::clipboard::MemoryClipboard) {
    let mut rt = Runtime::new(S { source }, view, (480.0, 800.0), Env::new(true, 2.0));
    rt.frame(0.0);
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    (rt, clipboard)
}

fn tick<T: 'static>(rt: &mut Runtime<T>) { rt.frame(1.0 / 60.0); }

fn copy<T: 'static>(rt: &mut Runtime<T>, clipboard: &rune::clipboard::MemoryClipboard, f: impl FnOnce(&mut rune::Tx)) -> String {
    rt.update(|_, tx| f(tx));
    tick(rt);
    rt.key(KeyEvent { key: Key::Command('c'), mods: Mods { cmd: true, ..Mods::default() } });
    tick(rt);
    clipboard.text().map(|t| t.to_string()).unwrap_or_default()
}

#[test]
fn copying_takes_the_source_of_whole_blocks() {
    let (mut rt, clipboard) = runtime(true);
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note")), SRC, "everything: the source verbatim");
    assert_eq!(rt.selected_text().as_deref().map(|t| t.starts_with("Title\n\nHello world")), Some(true), "selected as drawn");
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note/b1")), "Hello **world** and [link](https://x.dev/a).");
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note/b2")), "- alpha\n- [x] beta", "a whole list");
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note/b3")), "```rust\nfn main() {}\n```");
    // Part of a paragraph or of a list: as drawn.
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_text("note/b1", 6..11)), "world");
    let alpha = rt.elements().into_iter().find(|e| e.path.ends_with("note/b2/i0/#1/b0")).expect("alpha").path;
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text(alpha)), "alpha");
}

#[test]
fn a_drag_across_blocks_copies_each_whole() {
    let (mut rt, clipboard) = runtime(true);
    let find = |rt: &Runtime<S>, end: &str| rt.elements().into_iter().find(|e| e.path.ends_with(end)).unwrap_or_else(|| panic!("{end}"));
    let (para, last) = (find(&rt, "note/b1"), find(&rt, "note/b4"));
    let (x0, y0) = (para.rect[0] + 30.0, para.rect[1] + para.rect[3] / 2.0);
    let (x1, y1) = (last.rect[0] + 10.0, last.rect[1] + last.rect[3] / 2.0);
    rt.pointer_move(x0, y0);
    rt.pointer_down(x0, y0);
    tick(&mut rt);
    for k in 1..=6 {
        let f = k as f32 / 6.0;
        rt.pointer_move(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
        tick(&mut rt);
    }
    rt.pointer_up(x1, y1);
    tick(&mut rt);
    assert!(rt.selected_text().is_some_and(|t| t.contains("alpha") && !t.starts_with("Hello")), "{:?}", rt.selected_text());
    rt.key(KeyEvent { key: Key::Command('c'), mods: Mods { cmd: true, ..Mods::default() } });
    tick(&mut rt);
    assert_eq!(clipboard.text().as_deref().map(String::as_str), Some(&SRC[9..]).map(|s| s.trim_end()));
}

#[test]
fn without_copy_source_copying_takes_the_drawn_text() {
    let (mut rt, clipboard) = runtime(false);
    let all = copy(&mut rt, &clipboard, |tx| tx.select_all_text("note"));
    assert_eq!(all, "Title\n\nHello world and link.\n\n- alpha\n- [x] beta\n\nfn main() {}\n\nLast line.");
}

#[test]
fn a_whole_file_link_copies_its_reference_inside_a_paragraph() {
    let mut rt = Runtime::new((), |_, _| Markdown::new("see API [`main.rs`](src/main.rs#L12) here")
        .selectable(true)
        .link_copy_text(|url| Some(rune::LinkCopy::new(format!("[API]({url})")).including_prefix("API ")))
        .on_link(|_: &mut (), _| {})
        .build(&Kit::neutral(true)).key("note"), (360., 140.), Env::new(true, 1.));
    rt.frame(0.);
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    let text = rt.elements().into_iter().find(|e| e.path.ends_with("note/b0")).unwrap().text.unwrap();
    let start = text.find("main.rs").unwrap();
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_text("note/b0", start..start + 7)), "[API](src/main.rs#L12)");
    assert_eq!(rt.selected_text().as_deref(), Some("main.rs"));
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_text("note/b0", start..start + 4)), "main");
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note/b0")), "see [API](src/main.rs#L12) here");
}

#[test]
fn a_drag_starting_on_a_file_link_copies_it_and_a_click_still_opens_it() {
    #[derive(Default)]
    struct Page { editor: rune::TextEdit, opened: usize }
    let mut rt = Runtime::new(Page::default(), |s, _| col()
        .child(Markdown::new("[main.rs](src/main.rs) [i](tiny)rest").selectable(true)
            .link_copy_text(|href| Some(rune::LinkCopy::new(format!("[main.rs]({href})"))))
            .on_link(|s: &mut Page, _| s.opened += 1).build(&Kit::neutral(true)).key("reply"))
        .child(rune::text_input(&s.editor, rune::TextStyle::new(14., 400.)).key("editor")
            .on_edit(rune::edit_access(|s: &mut Page| &mut s.editor))), (360., 140.), Env::new(true, 1.));
    rt.frame(0.);
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    rt.update(|_, tx| tx.focus("editor", false));
    rt.frame(0.);
    let link = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("src/main.rs")).unwrap();
    let [x, y, w, h] = link.rect;
    let (left, right, cy) = (x + 0.5, x + w - 0.5, y + h / 2.);
    rt.pointer_down(left, cy);
    rt.pointer_move(right, cy);
    rt.pointer_up(right, cy);
    rt.frame(0.);
    assert_eq!(rt.selected_text().as_deref(), Some("main.rs"));
    assert_eq!(rt.state.opened, 0);
    rt.key(KeyEvent { key: Key::Command('c'), mods: Mods { cmd: true, ..Mods::default() } });
    for _ in 0..4 { rt.frame(1. / 60.); }
    assert_eq!(clipboard.text().as_deref().map(String::as_str), Some("[main.rs](src/main.rs)"));
    rt.pointer_down(left, cy);
    rt.pointer_up(left, cy);
    rt.frame(0.);
    assert_eq!(rt.state.opened, 1);
    rt.update(|_, tx| tx.focus("editor", false));
    rt.frame(0.);
    let tiny = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("tiny")).unwrap().rect;
    let mid = tiny[0] + tiny[2] / 2.;
    rt.pointer_down(mid - 1., cy);
    rt.pointer_move(mid + 1., cy);
    rt.pointer_up(mid + 1., cy);
    rt.frame(0.);
    assert_eq!(rt.selected_text().as_deref(), Some("i"));
    assert_eq!(rt.state.opened, 1, "a two-point drag selected the narrow glyph");
    rt.key(KeyEvent { key: Key::Command('c'), mods: Mods { cmd: true, ..Mods::default() } });
    for _ in 0..4 { rt.frame(1. / 60.); }
    assert_eq!(clipboard.text().as_deref().map(String::as_str), Some("[main.rs](tiny)"));
    let edge = tiny[0] + tiny[2];
    rt.pointer_down(edge - 0.5, cy);
    rt.pointer_up(edge - 0.5, cy);
    rt.frame(0.);
    rt.pointer_down(edge + 1., cy);
    rt.pointer_up(edge + 1., cy);
    rt.frame(0.);
    assert!(rt.selected_text().is_none(), "clicking a link does not begin a text double-click");
}

#[test]
fn a_cached_link_can_replace_its_tooltip_with_clipboard_text() {
    let mut rt = Runtime::new(false, |copying, _| {
        let md = Markdown::new("[label](target)").selectable(true).on_link(|_: &mut bool, _| {});
        let md = if *copying { md.link_copy_text(|_| Some(rune::LinkCopy::new("x"))) }
            else { md.link_help(|_| Some("x".into())).link_copy_text(|_| None) };
        md.build(&Kit::neutral(true)).key("note")
    }, (360., 140.), Env::new(true, 1.));
    rt.frame(0.);
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note/b0")), "label");
    rt.update(|copying, _| *copying = true);
    tick(&mut rt);
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note/b0")), "x");
}

#[derive(Default)]
struct Keys { takes: bool, keys: Vec<Key> }

fn short(_: &Keys, _: &mut Cx<Keys>) -> El {
    col().w_full().padding(10.0).child(Markdown::new("a\n\nb").selectable(true).build(&Kit::neutral(true)).key("note"))
}

#[test]
fn an_apps_on_key_sees_cmd_a_before_a_clicked_note() {
    for takes in [true, false] {
        let mut rt = Runtime::new(Keys { takes, keys: Vec::new() }, short, (300.0, 200.0), Env::new(true, 2.0));
        rt.on_key = Some(|s, ev, _| { s.keys.push(ev.key.clone()); s.takes && ev.key == Key::Command('a') });
        rt.frame(0.0);
        let a = rt.elements().into_iter().find(|e| e.path.ends_with("note/b0")).expect("the first paragraph");
        let (x, y) = (a.rect[0] + 3.0, a.rect[1] + a.rect[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        rt.key(KeyEvent { key: Key::Command('a'), mods: Mods { cmd: true, ..Mods::default() } });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.keys, [Key::Command('a')], "takes={takes}");
        let want = if takes { None } else { Some("a\n\nb") };
        assert_eq!(rt.selected_text().as_deref(), want, "takes={takes}: passed on, the note selects all");
    }
}

#[test]
fn inline_code_rule_and_crlf_blocks_copy_cleanly() {
    #[derive(Default)]
    struct Doc { src: &'static str }
    fn doc_view(s: &Doc, _: &mut Cx<Doc>) -> El {
        col().w_full().padding(10.0).child(Markdown::new(s.src).selectable(true).copy_source(true).build(&Kit::neutral(true)).key("note"))
    }
    let run = |src: &'static str| {
        let mut rt = Runtime::new(Doc { src }, doc_view, (480.0, 800.0), Env::new(true, 2.0));
        rt.frame(0.0);
        let clipboard = rune::clipboard::memory();
        rt.use_clipboard(&clipboard);
        (rt, clipboard)
    };

    // Document starting and ending with `---` rules returns the full source verbatim on Select All.
    let ruled = "---\n\nHello **world**\n\n---\n";
    let (mut rt, clipboard) = run(ruled);
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note")), ruled);

    // Double-clicking a single inline-code word selects the whole block despite uncopied padding.
    let inline = "`code`\n";
    let (mut rt, clipboard) = run(inline);
    let b0 = rt.elements().into_iter().find(|e| e.path.ends_with("note/b0")).expect("b0");
    let (x, y) = (b0.rect[0] + 12.0, b0.rect[1] + b0.rect[3] / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    tick(&mut rt);
    rt.key(KeyEvent { key: Key::Command('c'), mods: Mods { cmd: true, ..Mods::default() } });
    tick(&mut rt);
    assert_eq!(clipboard.text().as_deref().map(String::as_str), Some(inline));

    // CRLF code and diff blocks strip `\r` from drawn text while preserving source copy.
    let crlf = "```rust\r\nfn main() {}\r\n```\r\n\r\n```diff\r\n@@ -1 +1 @@\r\n-old\r\n+new\r\n```\r\n";
    let (mut rt, clipboard) = run(crlf);
    assert_eq!(copy(&mut rt, &clipboard, |tx| tx.select_all_text("note")), crlf);
    assert_eq!(rt.selected_text().as_deref(), Some("fn main() {}\n\n@@ -1 +1 @@\n-old\n+new"));
}
