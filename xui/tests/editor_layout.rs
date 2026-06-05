use xui::prelude::*;
use xui_interface::TextLayoutConstraints;

struct ZeroTextMeasurer;

impl xui::layout::TextMeasurer for ZeroTextMeasurer {
    fn measure_text(&mut self, _text: &str, _props: &ComputedTextStyle) -> Size<f32> {
        Size::<f32>::ZERO
    }

    fn measure_text_with_constraints(
        &mut self,
        _text: &str,
        _props: &ComputedTextStyle,
        _constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        Size::<f32>::ZERO
    }
}

#[test]
fn editor_shell_macro_layout_paints_sidebar_and_content_rects() {
    let mut app = App::new(|_| {
        xui! {
            <row size={Some(Size::fill())}>
                <container width={Sizing::fix(300.0)} height={Sizing::fill()} background={Color::BLACK} />
                <container size={Some(Size::fill())} background={Color::BLUE_500} />
            </row>
        }
    });
    let mut backend = MockRenderBackend::default();
    let mut measurer = ZeroTextMeasurer;

    app.resize(Size::<f32>::new(800.0, 600.0));
    app.render(&mut backend, &mut measurer).unwrap();

    let root = app.arena().root();
    let row_id = app.arena().children(root)[0];
    let pane_ids = app.arena().children(row_id);

    assert_eq!(
        app.arena().node(pane_ids[0]).unwrap().layout,
        Rect::new(0.0, 0.0, 300.0, 592.0)
    );
    assert_eq!(
        app.arena().node(pane_ids[1]).unwrap().layout,
        Rect::new(300.0, 0.0, 492.0, 592.0)
    );

    let painted_rects = backend
        .last_commands
        .iter()
        .filter_map(|command| match command {
            PaintCommand::Rect { rect, color, .. } => Some((*rect, *color)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(painted_rects.contains(&(
        Rect::new(0.0, 0.0, 300.0, 592.0),
        ComputedColorStyle::Solid(Color::BLACK),
    )));
    assert!(painted_rects.contains(&(
        Rect::new(300.0, 0.0, 492.0, 592.0),
        ComputedColorStyle::Solid(Color::BLUE_500),
    )));
}
