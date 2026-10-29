use std::time::Instant;

use xui_core::prelude::{
    BlockChange, SurfaceChanges, SurfaceInput, SurfacePosition, TextRun, TextRunStyle,
    TextSurfaceModel,
};
use xui_interface::events::{RawIme, TextPayload};
use xui_interface::{ClipboardContent, PlatformReply, RequestId, TextOffset, TextRange};

use super::*;

/// A host with no layout: vertical moves fall back to the document's ends.
#[derive(Default)]
struct Host {
    clipboard: Option<String>,
    changed: usize,
}

impl EditorHost for Host {
    fn vertical_move(&self, _: SurfacePosition, _: f32, _: i32) -> Option<SurfacePosition> {
        None
    }

    fn caret_x(&self, _: SurfacePosition) -> Option<f32> {
        None
    }

    fn page_lines(&self) -> i32 {
        10
    }

    fn reveal(&mut self, _: SurfacePosition) {}

    fn changed(&mut self) {
        self.changed += 1;
    }

    fn write_clipboard(&mut self, text: String) -> bool {
        self.clipboard = Some(text);
        true
    }

    fn read_clipboard(&mut self) -> Option<RequestId> {
        Some(RequestId(7))
    }

    fn capture_pointer(&mut self) {}

    fn release_pointer_capture(&mut self) {}
}

fn view(text: &str) -> EditorView {
    EditorView::new(&BufferHandle::new(text))
}

fn type_text(view: &mut EditorView, host: &mut Host, text: &str) {
    for ch in text.chars() {
        view.handle(SurfaceInput::Text(&ch.to_string()), host);
    }
}

fn caret(view: &EditorView) -> Pos {
    view.selections().primary().head
}

fn to_end(view: &mut EditorView, host: &mut Host) {
    view.run(
        EditorCommand::Move {
            motion: Motion::DocumentEnd,
            extend: false,
        },
        host,
    );
}

#[test]
fn typing_is_one_undo_step_and_redo_brings_it_back() {
    let mut view = view("hello");
    let mut host = Host::default();
    to_end(&mut view, &mut host);
    type_text(&mut view, &mut host, " world");
    assert_eq!(view.text(), "hello world");
    view.run(EditorCommand::Undo, &mut host);
    assert_eq!(view.text(), "hello");
    assert_eq!(caret(&view), Pos::new(0, 5));
    view.run(EditorCommand::Redo, &mut host);
    assert_eq!(view.text(), "hello world");
    assert!(host.changed > 0);
}

#[test]
fn every_caret_types_and_ends_after_its_text() {
    let mut view = view("ab\ncd\nef");
    let mut host = Host::default();
    view.set_selections(Selections::from_list(
        vec![
            Selection::caret(Pos::new(0, 1)),
            Selection::caret(Pos::new(1, 1)),
            Selection::caret(Pos::new(2, 2)),
        ],
        0,
    ));
    type_text(&mut view, &mut host, "XY");
    assert_eq!(view.text(), "aXYb\ncXYd\nefXY");
    let carets: Vec<Pos> = view
        .selections()
        .iter()
        .map(|selection| selection.head)
        .collect();
    assert_eq!(carets, [Pos::new(0, 3), Pos::new(1, 3), Pos::new(2, 4)]);
}

#[test]
fn a_newline_keeps_the_indent_and_adds_a_block() {
    let mut view = view("    if x {");
    let mut host = Host::default();
    let _ = view.take_changes();
    to_end(&mut view, &mut host);
    view.run(EditorCommand::Newline, &mut host);
    assert_eq!(view.text(), "    if x {\n    ");
    assert_eq!(caret(&view), Pos::new(1, 4));
    let SurfaceChanges::Edits(changes) = view.take_changes() else {
        panic!("expected edits");
    };
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, BlockChange::Inserted { index: 1, .. }))
    );
}

#[test]
fn backspace_at_a_line_start_joins_the_lines() {
    let mut view = view("one\ntwo");
    let mut host = Host::default();
    let _ = view.take_changes();
    let second = view.buffer().read(|buffer| buffer.document().id(1));
    view.set_selections(Selections::single(Selection::caret(Pos::new(1, 0))));
    view.run(EditorCommand::Backspace, &mut host);
    assert_eq!(view.text(), "onetwo");
    assert_eq!(caret(&view), Pos::new(0, 3));
    let SurfaceChanges::Edits(changes) = view.take_changes() else {
        panic!("expected edits");
    };
    assert!(changes.contains(&BlockChange::Removed { id: second }));
}

fn preedit(text: &str) -> RawIme {
    RawIme::Preedit {
        text: TextPayload::new(text),
        cursor: Some(TextRange::new(
            TextOffset::byte_offset(text.len()),
            TextOffset::byte_offset(text.len()),
        )),
        timestamp: Instant::now(),
    }
}

#[test]
fn a_composition_is_not_history_but_its_commit_is() {
    let mut view = view("ab");
    let mut host = Host::default();
    view.set_selections(Selections::single(Selection::caret(Pos::new(0, 1))));
    view.handle(SurfaceInput::Ime(&preedit("n")), &mut host);
    view.handle(SurfaceInput::Ime(&preedit("ni")), &mut host);
    assert_eq!(view.text(), "anib");
    assert_eq!(caret(&view), Pos::new(0, 3));
    view.handle(
        SurfaceInput::Ime(&RawIme::Commit {
            text: TextPayload::new("你"),
            timestamp: Instant::now(),
        }),
        &mut host,
    );
    assert_eq!(view.text(), "a你b");
    view.run(EditorCommand::Undo, &mut host);
    assert_eq!(view.text(), "ab");
    assert!(!view.can_undo());
}

#[test]
fn a_paste_spreads_its_lines_over_as_many_carets() {
    let mut view = view("1\n2");
    let mut host = Host::default();
    view.set_selections(Selections::from_list(
        vec![
            Selection::caret(Pos::new(0, 1)),
            Selection::caret(Pos::new(1, 1)),
        ],
        0,
    ));
    view.run(EditorCommand::Paste, &mut host);
    let reply = PlatformReply::Clipboard(Some(ClipboardContent::Text("a\nb".to_owned())));
    view.handle(SurfaceInput::PlatformReply(RequestId(7), &reply), &mut host);
    assert_eq!(view.text(), "1a\n2b");
}

#[test]
fn copy_and_cut_go_through_the_host() {
    let mut view = view("keep cut");
    let mut host = Host::default();
    view.set_selections(Selections::single(Selection::new(
        Pos::new(0, 5),
        Pos::new(0, 8),
    )));
    view.run(EditorCommand::Cut, &mut host);
    assert_eq!(host.clipboard.as_deref(), Some("cut"));
    assert_eq!(view.text(), "keep ");
}

#[test]
fn indenting_a_selection_indents_its_lines_and_outdent_reverts() {
    let mut view = view("a\nb\nc");
    let mut host = Host::default();
    view.set_selections(Selections::single(Selection::new(
        Pos::new(0, 0),
        Pos::new(1, 1),
    )));
    view.run(EditorCommand::Indent, &mut host);
    assert_eq!(view.text(), "    a\n    b\nc");
    assert_eq!(view.selections().primary().end(), Pos::new(1, 5));
    view.run(EditorCommand::Outdent, &mut host);
    assert_eq!(view.text(), "a\nb\nc");
}

#[test]
fn deleting_words_and_moving_by_them() {
    let mut view = view("let value = 1;");
    let mut host = Host::default();
    view.set_selections(Selections::single(Selection::caret(Pos::new(0, 9))));
    view.run(EditorCommand::DeleteWordBackward, &mut host);
    assert_eq!(view.text(), "let  = 1;");
    view.run(
        EditorCommand::Move {
            motion: Motion::WordRight,
            extend: true,
        },
        &mut host,
    );
    assert_eq!(view.selections().primary().end(), Pos::new(0, 8));
}

#[test]
fn read_only_ignores_edits_but_moves_and_copies() {
    let mut view = view("fixed");
    let mut host = Host::default();
    view.set_read_only(true);
    assert_eq!(
        view.handle(SurfaceInput::Text("x"), &mut host),
        EventResult::Ignored
    );
    view.run(EditorCommand::SelectAll, &mut host);
    view.run(EditorCommand::Copy, &mut host);
    assert_eq!(host.clipboard.as_deref(), Some("fixed"));
    assert_eq!(
        view.run(EditorCommand::Backspace, &mut host),
        EventResult::Ignored
    );
    assert_eq!(view.text(), "fixed");
}

#[test]
fn blocks_are_built_once_until_their_line_changes() {
    let mut view = view("a\nb");
    view.buffer().set_highlighter(Some(Box::new(|line: &str| {
        vec![TextRun::new(0..line.len(), TextRunStyle::new())]
    })));
    let id = view.buffer().read(|buffer| buffer.document().id(0));
    let first = view.block(id);
    assert_eq!(first.text.as_str(), "a");
    assert_eq!(first.runs.len(), 1);
    let mut host = Host::default();
    view.set_selections(Selections::single(Selection::caret(Pos::new(0, 1))));
    type_text(&mut view, &mut host, "z");
    assert_eq!(view.block(id).text.as_str(), "az");
}

// ---- views sharing a buffer ----------------------------------------------

#[test]
fn an_edit_in_one_view_moves_the_carets_of_another() {
    let buffer = BufferHandle::new("first\nsecond");
    let mut left = EditorView::new(&buffer);
    let mut right = EditorView::new(&buffer);
    let mut host = Host::default();
    let _ = (left.take_changes(), right.take_changes());

    right.set_selections(Selections::single(Selection::caret(Pos::new(1, 3))));
    left.set_selections(Selections::single(Selection::caret(Pos::new(0, 0))));
    type_text(&mut left, &mut host, "new\n");
    assert_eq!(buffer.text(), "new\nfirst\nsecond");

    right.catch_up();
    assert_eq!(caret(&right), Pos::new(2, 3), "followed its line down");
    // The other view's surface hears of the new line too.
    let SurfaceChanges::Edits(changes) = right.take_changes() else {
        panic!("expected edits");
    };
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, BlockChange::Inserted { index: 1, .. }))
    );
}

#[test]
fn undo_in_another_view_reverts_the_step_and_restores_only_its_maker() {
    let buffer = BufferHandle::new("abc");
    let mut maker = EditorView::new(&buffer);
    let mut other = EditorView::new(&buffer);
    let mut host = Host::default();
    maker.set_selections(Selections::single(Selection::caret(Pos::new(0, 1))));
    other.set_selections(Selections::single(Selection::caret(Pos::new(0, 3))));
    type_text(&mut maker, &mut host, "XY");
    assert_eq!(buffer.text(), "aXYbc");

    other.run(EditorCommand::Undo, &mut host);
    assert_eq!(buffer.text(), "abc");
    // The view that undid it followed the edit back; the maker's caret goes
    // to where the step left it once it catches up, mapped through the undo.
    assert_eq!(caret(&other), Pos::new(0, 3));
    maker.catch_up();
    assert_eq!(caret(&maker), Pos::new(0, 1));

    maker.run(EditorCommand::Redo, &mut host);
    assert_eq!(buffer.text(), "aXYbc");
    assert_eq!(
        caret(&maker),
        Pos::new(0, 3),
        "the maker gets its own carets back"
    );
}

#[test]
fn application_edits_are_steps_every_view_sees() {
    let buffer = BufferHandle::new("body");
    let mut view = EditorView::new(&buffer);
    let mut host = Host::default();
    let _ = view.take_changes();
    to_end(&mut view, &mut host);

    buffer.transact(|tx| {
        tx.insert(Pos::new(0, 0), "// header\n");
        tx.set_kind(EditKind::Other);
    });
    view.catch_up();
    assert_eq!(caret(&view), Pos::new(1, 4));
    assert!(matches!(view.take_changes(), SurfaceChanges::Edits(_)));

    view.run(EditorCommand::Undo, &mut host);
    assert_eq!(buffer.text(), "body");
}

#[test]
fn replacing_the_text_resets_every_view() {
    let buffer = BufferHandle::new("old text");
    let mut view = EditorView::new(&buffer);
    let _ = view.take_changes();
    view.set_selections(Selections::single(Selection::caret(Pos::new(0, 4))));
    buffer.set_text("new");
    assert_eq!(view.take_changes(), SurfaceChanges::Reset);
    assert_eq!(caret(&view), Pos::new(0, 0));
    assert!(!view.can_undo());
}

#[test]
fn a_composition_in_one_view_follows_edits_from_another() {
    let buffer = BufferHandle::new("xy");
    let mut composing = EditorView::new(&buffer);
    let mut other = EditorView::new(&buffer);
    let mut host = Host::default();
    composing.set_selections(Selections::single(Selection::caret(Pos::new(0, 2))));
    composing.handle(SurfaceInput::Ime(&preedit("ka")), &mut host);
    assert_eq!(buffer.text(), "xyka");

    other.set_selections(Selections::single(Selection::caret(Pos::new(0, 0))));
    type_text(&mut other, &mut host, ">>");
    assert_eq!(buffer.text(), ">>xyka");

    composing.handle(
        SurfaceInput::Ime(&RawIme::Commit {
            text: TextPayload::new("か"),
            timestamp: Instant::now(),
        }),
        &mut host,
    );
    assert_eq!(buffer.text(), ">>xyか");
}
