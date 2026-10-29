# Text surface: an extension point for editors

Status: **phases 0 to 3 are implemented; their tests are written but have not
been run yet.** This is what `xui` itself provides so that a rich-text or code
editor can live in a crate of its own, `crates/xui-editor`, the way
`xui-table` sits on top of `xui`. Where the code differs from the sketches
below, see [What was built](#what-was-built).

## Why

An editor needs four things from the framework that an application crate cannot
build today:

1. **Text with more than one style in a paragraph.** `TextLayoutInput` carries a
   single `default_style` (`xui-interface/src/text.rs`), and `TextPaintStyle` a
   single `color` (`xui-interface/src/render.rs`). `TextSpan` and `TextStyleId`
   are declared, and `GlyphRun::style_id` exists, but nothing produces or reads
   them. Syntax highlighting, bold/italic, links and inline code all need this.
2. **A widget that owns many text units.** A host widget has one text slot,
   `TextLayoutSlot::PRIMARY`, which taffy measures and the paint pass draws. The
   canvas owns more (`canvas_text_slot`), but only as a special case in the paint
   pass, and `CanvasTextLayout` gives back metrics, not geometry. A document of
   10^5 lines cannot be one paragraph either: xui-f keys its prepared layout on
   the whole text (`HardLayoutKey`), so every keystroke reshapes every line.
3. **Raw input, IME and the platform, from outside `xui-core`.** The widget roles
   (`PaintContent`, `TextSource`, `HandleEvent`, `PlatformTextInput`, in
   `widgets/roles.rs`) are `pub(crate)`, and host widgets are a closed set
   (`define_widgets!`). An external crate can only build a canvas, which sees no
   raw keys, opens no IME session and cannot hit-test its text.
4. **Virtualized vertical layout with variable heights.** `virtual_list` assumes
   fixed row heights. `TextHost` already has the right shape for this, a
   *virtual document* (`TextDocumentId`, `ParagraphId`, a Fenwick height index,
   `paragraph_at_y`, `set_visible_paragraphs`), but nothing outside its own tests
   uses it.

`TextInputWidget` shows what happens without an extension point: its value model,
keymap, undo history and IME handling are all inside `xui-core`, so turning it
multi-line, or adding word movement, means editing the core.

## The split

| Lives in | What | Why there |
|---|---|---|
| `xui-interface` | style runs in `TextLayoutInput`, paint runs and decorations in `TextPaintProps` | every text and render backend implements them |
| `xui-core` | the `TextSurface` host widget, the `TextSurfaceModel` trait, geometry queries, the IME session | it touches the layout and paint passes, `TextHost`, phase guards and event routing |
| `xui-editor` (new crate) | document model (rope plus marks or blocks), selection, commands, keymap, undo grouping, highlighting, serialization | policy: every editor makes different choices, and the dependencies (tree-sitter, parsers) are heavy |

One narrow, purpose-built host widget, not a public "custom widget" API. Making
the roles public would freeze the internal widget representation (`Widgets`, the
capability columns, `WidgetInstance`) as API, and would let outside code break
the runtime's invariants: live-id preconditions, `SYNC_TRANSFORM`, phase guards.
A dedicated widget keeps those inside `xui-core` and gives the outside one trait.
Whether a general custom-widget API is wanted can be decided later, with this as
the first real user.

## 1. Text contract: shaping runs and paint runs

Two kinds of attribute, kept apart because they invalidate different things:

- **Shaping attributes** change glyphs and advances: family, size, weight, style,
  stretch, letter spacing, OpenType features. They are part of the layout input
  and its key.
- **Paint attributes** change only how glyphs are drawn: color, background,
  underline, strikethrough. They are part of the paint props. Re-highlighting a
  file after a parse must not reshape it.

```rust
// xui-interface/src/text.rs
pub struct TextLayoutInput {
    pub text: TextContent,
    /// Byte ranges over `text`, sorted, non-overlapping. Gaps use
    /// `default_style`. Empty for single-style text: the common case stays free.
    pub runs: Arc<[ShapeRun]>,
    pub constraints: TextLayoutConstraints,
    pub default_style: ComputedTextStyle,
    // ... unchanged
}

pub struct ShapeRun {
    pub range: Range<usize>,
    pub style: ShapeStyle, // the font fields of ComputedTextStyle, plus features
}

// xui-interface/src/render.rs
pub struct TextPaintProps {
    pub style: TextPaintStyle,         // the default, as now
    pub runs: Arc<[PaintRun]>,         // byte ranges, sorted, non-overlapping
    pub decorations: Arc<[TextDecor]>, // ranges that may overlap; see below
    pub caret: Option<TextCaret>,
    pub selection: Option<TextSelectionPaint>,
    pub ime: Option<TextImePaint>,
}

pub struct PaintRun {
    pub range: Range<usize>,
    pub color: Color,
    pub decoration: TextDecoration,
}
```

Rules:

- **Ranges are byte offsets** into the paragraph's text, because the model is a
  rope or a `str` and a parser reports bytes. The existing `TextOffset` keeps its
  unit for queries; runs do not need one.
- **`GlyphRun::style_id` is the index of the `ShapeRun`** the glyphs came from
  (`u32::MAX` for the default style). `TextSpan` is removed; it is the old,
  unused spelling of the same idea.
- **Paint runs apply per cluster.** A ligature or a combining sequence across a
  color boundary takes the color of the cluster's first byte, as browsers do.
  Renderers get the cluster from `GlyphInstance::cluster` and look the run up
  once per cluster, not once per glyph.
- **Shaping does not cross a shaping-run boundary**: the shaper itemizes by run
  as it already itemizes by font fallback and bidi level. Paint runs never split
  shaping, so highlighting cannot break ligatures.
- The `Text` host widget gains an attributed form (`text(...).runs(...)`), so
  styled labels work without the surface.

Backend work: xui-f itemizes `shape_line` by run, with a `LineFonts` per distinct
shaping style; xui-cosmic maps runs to cosmic-text's `AttrsList`. Vello and Skia
take the color per cluster in their glyph drawing. `xui-wgpu-text` already has a
color per glyph instance in `text.wgsl`; only `GlyphRunDraw`, which takes one
`color` per run, needs to accept colors per glyph, or the caller splits runs at
color boundaries.

## 2. The `TextSurface` host widget

A focusable viewport that shows a virtual document: a list of **blocks**, each
one paragraph of shaped text. The widget's size comes from style, like a canvas.
Its content is not taffy children, and taffy never measures the blocks: the
surface lays out the visible ones itself, after layout, as the canvas lays out
its text in the paint sync.

```rust
define_widgets! {
    // ...
    TextSurface => TextSurface [paint, events, text_input, default_focus] via state,
}

#[widget(style(box_size, margin, padding, position, display, typography, visibility,
               transform, effects, cursor),
         behavior(focus, accessibility, shortcuts, motion, focus_binding,
                  size_observation, events))]
pub struct TextSurface {
    pub model: TextSurfaceHandle,
    #[default] pub disabled: bool,
    #[default] pub wrap: SurfaceWrap, // Width (soft-wrap to the viewport) | None
    #[default] pub overscan: f32,     // in viewports; at least 1.0
}
```

`TextSurfaceHandle` follows `CanvasController`: a cloneable handle compared by
identity, with an invalidator bound to every surface that shows it. Whoever owns
the handle, whether the editor crate, an async highlighter or the application,
calls `invalidate()` after changing the model, and the bound surfaces re-sync on
the next frame without a component re-render.

### Blocks and the document cache

Each surface owns one `TextDocumentId` in `TextHost`, with one `ParagraphId` per
block. Blocks have **stable ids chosen by the model** (`BlockId(u64)`), so an
insertion above the viewport does not reshape what is on screen.

The surface's sync step, run during the paint sync:

1. Drain the model's change list (below) into the document: `insert_paragraph`,
   `remove_paragraph`, and invalidation for changed text or shaping style.
   Changed paint runs only mark the block for repaint.
2. From `scroll_y` and the height index, find the visible range plus overscan;
   `set_visible_paragraphs`.
3. Shape every block in that range that has no layout for the current wrap width,
   and record its real height (`update_paragraph_height`). Blocks outside the
   range keep an estimate (line count times line height, from the model).
4. **Anchor the scroll.** When heights above the viewport change, shift
   `scroll_y` by the same amount so the top visible block stays put.
5. Emit one `Primitive::Text` per visible block, each with its paint runs, plus
   decorations, under the surface's clip and scroll transform.

`TextPrimitive::slot` grows into a source that also names a paragraph
(`TextUnitRef::Slot(TextLayoutSlot) | Paragraph(TextDocumentId, ParagraphId)`),
so the renderer can resolve a block's layout the way it resolves a slot now.

Each block is its own text unit with its own shaper state. The whole-text
invalidation in xui-f stops mattering, because a keystroke changes one block.

Two costs in the document cache need fixing before large files work. `order` is
a `Vec<ParagraphId>`: inserting into it rebuilds the whole Fenwick index
(`rebuild_height_index`), and `paragraph_top` finds a paragraph by a linear
scan. Both are O(blocks). They need a chunked or balanced sequence that keeps
heights summed per node, giving O(log n) insert, remove, index lookup and y
lookup.

### The model trait

Implemented by the editor crate. Reads take `&self` and happen while the
runtime renders; only `handle_input` mutates, and it runs in the event phase.

```rust
pub trait TextSurfaceModel {
    /// What changed since the last call. The surface calls it once per sync.
    /// The first call after binding returns `Reset`.
    fn take_changes(&mut self) -> SurfaceChanges;

    fn block_count(&self) -> usize;
    fn block_at(&self, index: usize) -> BlockId;
    fn block(&self, id: BlockId) -> SurfaceBlock;

    /// Selections, carets, diagnostics, find matches; see "Decorations".
    fn decorations(&self, visible: Range<usize>) -> Vec<SurfaceDecor>;

    fn handle_input(&mut self, input: SurfaceInput, cx: &mut SurfaceCx<'_>) -> EventResult;
}

pub enum SurfaceChanges {
    None,
    Reset,
    Edits(Vec<BlockChange>), // Inserted { index, id } | Removed { id }
                             // | Text { id } | Paint { id }
}

pub struct SurfaceBlock {
    pub text: TextContent,              // one paragraph, no trailing break
    pub shape_runs: Arc<[ShapeRun]>,
    pub paint_runs: Arc<[PaintRun]>,
    pub paragraph: ParagraphStyle,      // alignment, indent, spacing
    pub text_revision: u64,             // bumps on text or shaping-run change
    pub paint_revision: u64,
    pub estimated_lines: u32,
}
```

`take_changes` takes `&mut self`, and the surface calls it during the paint sync.
That does not break the phase guard, which covers hook and `slot` storage only,
not a model the application owns behind a `RefCell`. But the model must not
write reactive signals from it. The trait docs say so, and a debug build can
assert the `RenderPhase` when the model calls back into `xui`.

### Input

The surface turns events into `SurfaceInput` and hands them to the model. It
resolves pointer positions to text positions, because only the surface has the
layout. Key handling, commands and their meaning stay with the model.

```rust
pub enum SurfaceInput<'a> {
    Pointer { kind: PointerKind, at: SurfacePosition, point: Point,
              modifiers: Modifiers, click_count: u8 },
    Key(&'a RawKeyboard),
    Text(&'a str),                    // committed text outside a composition
    Ime(&'a RawIme),
    Command(CommandId),               // COPY, PASTE, UNDO, ... from menus or bindings
    PlatformReply(RequestId, &'a PlatformReply),
    Focus(bool),
}

pub struct SurfacePosition { pub block: BlockId, pub offset: usize, pub affinity: Affinity }
```

`SurfaceCx` wraps `EventContext` with what the model needs and nothing more:

- **Geometry, for resident blocks only**: `hit_test(point)`, `caret_rect(pos)`,
  `selection_rects(range)`, `block_top(id)`, `line_bounds(pos)`, and
  `vertical_move(pos, goal_x, lines)` for up/down and page movement. A query on a
  block that is not shaped returns `None`. Overscan of at least one viewport on
  each side means any line or page move from a visible caret lands on a resident
  block. Jumps that do not need geometry, such as Ctrl+End, are offset arithmetic
  in the model followed by `reveal`.
- **Viewport**: `viewport()`, `scroll_to(y)`, `reveal(pos, margin)`.
- **Platform**: `send_platform` and `request_platform`, as on `EventContext`.
  Replies come back as `SurfaceInput::PlatformReply`.
- **Invalidation**: `changed()`, which is what `invalidate()` does on the handle.

The surface itself handles the wheel and touchpad scroll (unless the model
consumes the event), the caret blink (wakes, as `TextInputWidget` does now) and
pointer capture during a drag selection. It also hides carets and the IME
underline when it loses focus.

### Decorations

Everything drawn over or under text, other than glyph color, is one list:

```rust
pub struct SurfaceDecor {
    pub range: SurfaceRange, // start and end SurfacePosition; may span blocks
    pub kind: DecorKind,     // Background(Color) | Underline { color, style: Solid | Wavy | Dotted }
                             // | Caret { color, width } | Outline(Color)
    pub layer: DecorLayer,   // BelowText | AboveText
}
```

The surface turns a range into rects with `selection_rects`, clipped to resident
blocks. Selections (several, for multiple cursors), the current-line highlight,
find matches, diagnostics squiggles and the IME composition underline are all
decorations, so the model never computes pixel geometry for them.

### IME

Phase one keeps today's contract: preedit, commit and a `cursor_area`. The
surface implements `PlatformTextInput` with `multiline: true` and asks the model
for its primary caret to place the candidate window. The model treats preedit
text as a composing edit and draws the underline as a decoration, the way
`TextController` keeps a composing range now.

The xui-macos host answers `selectedRange` and `attributedSubstring...` with
"not found", since the runtime does not share its document (`view.rs`). That rules
out reconversion, accent press-and-hold over existing text, and accurate
`firstRectForCharacterRange` for ranges other than the caret. Sharing the
document is the "text document" entry in the State channel of
`docs/shell-api.md`, and belongs to a later phase. It needs the model to answer
"text around the selection" and "rect of range" queries, which the trait above
can grow without breaking.

## 3. What the editor crate builds on it

Outside `xui`, and not part of this proposal beyond showing the interface is
enough:

- A rope document with block structure: lines for code, paragraphs and list
  items for prose. It maps rope edits to `BlockChange`s.
- Selections as ranges over the rope, multi-cursor, a goal column for vertical
  moves, word and grapheme boundaries (unicode-segmentation).
- A keymap and command table, platform-aware like `TextKeymap` but configurable.
- Undo with grouping by time and edit kind, and selection restore.
- Highlighting (tree-sitter or a simpler tokenizer) that writes paint runs and
  reports `BlockChange::Paint`, possibly from a background task whose result
  comes back through `App`'s async channel and then calls `invalidate()`.
- A gutter: line numbers as a second, narrow `TextSurface` bound to the same
  scroll, or as a decoration kind. See open questions.

## Rollout

Each phase lands on its own and is testable without the next.

0. **Runs.** `ShapeRun`/`PaintRun` in the interface; xui-f, xui-cosmic, Vello,
   Skia and `xui-wgpu-text` implement them; `Text` takes runs. Snapshot tests of
   mixed-style paragraphs through Skia. Delete `TextSpan`.
1. **Read-only surface.** `TextSurface`, the handle, the model trait without
   input, document-cache wiring and its O(log n) sequence, scroll and anchoring,
   decorations. A viewer for a 10^5-line file in the example app, with a bench in
   the style of `frame_bench` (a test, not a gate) that measures sync cost per
   scrolled frame and per single-block edit.
2. **Input.** `SurfaceInput`, `SurfaceCx` geometry, blink, IME under the
   current contract, platform requests.
3. **`xui-editor`** as a workspace member, with a code-editor demo bin in
   `xui-example-app`.
4. **Later.** Document-sharing IME (with `xui-macos` first), accessibility text,
   and `TextInputWidget` rewritten as a one-block surface with a small built-in
   model, so single-line and multi-line editing share one path.

## What was built

Where the implementation went another way than the sketches above:

- **Blocks carry `TextRuns<TextRun>`, not resolved shape and paint runs.** The
  surface splits them against its computed style, so a model writes one list
  of overrides. A recolor still does not reshape: the split's shape runs come
  out equal, and so does the layout key. `SurfaceBlock` has no revisions; the
  surface reads the visible blocks again on every sync and the layout cache
  is keyed by content.
- **`decorations` takes the visible block ids**, and the model also answers
  `primary_caret` (for the IME) and `estimated_lines`.
- **The document cache is an implicit treap** (`xui-core/src/text/sequence.rs`)
  with subtree counts and heights and parent links: insert, remove, index,
  top and y lookup are all O(log n).
- **`TextPrimitive::slot` became `unit: TextUnitRef`**, a slot or a document
  paragraph; `TextHost::active_layout` resolves either.
- **Events reach the model with `EventContext::text_units`**, a
  `TextUnitLookup` over the text host. `TextLayoutQuery` gained byte-offset
  queries (`hit_test_byte`, `caret_rect_at_byte`) and `line_rects`.
- **The IME session takes an optional primary layout**, since a surface has
  none; the surface places the candidate window from the primary caret it
  resolved while syncing.
- **A wheel a widget consumes no longer scrolls the containers around it.**
- **Paint spans are decided per glyph run.** A run whose text range lies in
  one paint run, or in a gap, is one span; only runs a boundary crosses are
  walked, a cluster at a time, with a cursor over the paint runs that moves
  either way, so right-to-left runs cost the same.
- **The editor crate is testable without a surface**: a view edits through
  an `EditorHost` trait, which `SurfaceCx` implements.
- **The editor is a shared `Buffer` and per-surface `EditorView`s.** The
  buffer holds the text, history, highlighter and a log of edits; every edit,
  a view's or the application's, is a `Transaction`. Each view reads the log
  from its own revision: its surface gets the changed blocks, its carets and
  IME composition are mapped through the edits. Undo is the buffer's; the
  view that made a step gets its selections back. The buffer tells views
  through `WeakTextSurfaceHandle`s, so a view and its buffer do not keep each
  other alive.

## Not in scope

- Inline objects (images, widgets inside a line). `ShapeRun` leaves room for a
  placeholder kind of run with a fixed advance, but it is not designed here.
- Columns, floats and other page layout. Blocks stack vertically.
- Collaborative editing. That belongs to the model and does not touch the surface.

## Open questions

- **Horizontal scroll without wrap.** For code, the surface needs `scroll_x` and
  the widest resident line. Tracking the widest line over the whole document
  either needs every block shaped once or settles for a running maximum. The
  proposal is the running maximum, as most editors do.
- **The gutter.** A second surface keeps the surface simple but needs two
  surfaces to share a scroll. A `Gutter` decoration kind keeps one surface but
  puts layout policy in the core. Leaning towards the second surface, with a
  `scroll_link` prop.
- **Scrollbars.** The surface can reuse `scrollbar_parts` from `ui_runtime/scroll.rs`
  with its own extent. It is not a runtime scroll container, so how the two
  share hit-testing and styling is unsettled.
- **Where the model lives on threads.** The trait as written is `!Send` and runs
  on the UI thread. A background highlighter hands results back through the
  async channel. Whether anything must be `Send` is left to the editor crate.
