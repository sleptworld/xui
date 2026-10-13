use xui_core::render::{
    FrameBuilder, FrameProperties, Primitive, RenderScene, SceneCompiler, Shape, ShapePrimitive,
};
use xui_interface::{Affine, Bounds, Color, ComputedColorStyle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut scene = RenderScene::new();
    let space = scene.insert_transform(Affine::IDENTITY);
    let node = scene.insert_primitive(Primitive::Shape(ShapePrimitive {
        bounds: Bounds::from_origin_size((0.0, 0.0), (40.0, 20.0)),
        shape: Shape::Rect,
        fill: Some(ComputedColorStyle::Solid(Color::BLACK)),
        stroke: None,
        shadow: None,
    }));
    scene.append_child(scene.root(), space)?;
    scene.set_child(space, Some(node))?;
    let mut compiler = SceneCompiler::new();
    let snapshot = scene.dirty_snapshot();
    compiler.compile(&scene, &snapshot)?;
    let compiled = compiler.compiled_scene().unwrap();
    let revision = compiled.scene_revision();
    let mut properties = FrameProperties::default();
    properties.set_transform(space, Affine::translate(30.0, 0.0));
    properties.validate(&scene)?;
    let mut builder = FrameBuilder::new();
    let viewport = Bounds::from_origin_size((0.0, 0.0), (200.0, 100.0));
    let frame = builder.build(compiled, viewport, &properties)?;
    assert_eq!(frame.scene_revision, revision);
    assert_eq!(frame.layers[0].items.len(), 1);
    Ok(())
}
