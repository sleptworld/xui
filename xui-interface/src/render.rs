use crate::{
    Color, ComputedColorStyle, ComputedShadowStyle, ComputedStrokeStyle, NodeId,
    NodeLifecycleEvent, Point, Rect, Size, TextProps, Translation,
};

pub trait Painter {
    fn push(&mut self, command: PaintCommand);
}

impl Painter for Vec<PaintCommand> {
    fn push(&mut self, command: PaintCommand) {
        Vec::push(self, command);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintCommand {
    Rect {
        rect: Rect,
        color: ComputedColorStyle,
        stroke: Option<ComputedStrokeStyle>,
        shadow: Option<ComputedShadowStyle>,
    },
    RoundedRect {
        rect: Rect,
        radius: f32,
        color: ComputedColorStyle,
        stroke: Option<ComputedStrokeStyle>,
        shadow: Option<ComputedShadowStyle>,
    },
    Line {
        from: Point,
        to: Point,
        color: Color,
        width: f32,
    },
    Text(TextPaintCommand),
    // Clip
    PushClip(Rect),
    PopClip,

    // Transform
    PushTransform {
        translate: Translation,
    },
    PopTransform,

    Clear(Color),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPaintCommand {
    pub node_id: NodeId,
    pub rect: Rect,
    pub props: TextProps,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DamageRegion {
    rects: Vec<Rect>,
}

impl DamageRegion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn full(size: Size<f32>) -> Self {
        let mut region = Self::new();
        region.add(Rect::new(0.0, 0.0, size.width, size.height));
        region
    }

    pub fn add(&mut self, rect: Rect) {
        if rect.width > 0.0 && rect.height > 0.0 {
            self.rects.push(rect);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    pub fn intersects(&self, rect: Rect) -> bool {
        self.rects.iter().any(|damage| damage.intersects(rect))
    }
}

pub trait RenderBackend<T> {
    type Error;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error>;
    fn paint(
        &mut self,
        commands: &[PaintCommand],
        damage: &DamageRegion,
        text: &mut T,
    ) -> Result<(), Self::Error>;
    fn end_frame(&mut self) -> Result<(), Self::Error>;

    fn did_present(&self) -> bool {
        true
    }

    fn resize(&mut self, _size: Size<f32>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_factor(&mut self, _factor: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

pub trait DrawBackend<T>: RenderBackend<T> {}

impl<T, B: RenderBackend<T>> DrawBackend<T> for B {}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MockRenderBackend {
    pub frame_size: Option<Size<f32>>,
    pub frames: usize,
    pub last_damage: DamageRegion,
    pub last_commands: Vec<PaintCommand>,
}

impl<T> RenderBackend<T> for MockRenderBackend {
    type Error = core::convert::Infallible;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.frame_size = Some(size);
        Ok(())
    }

    fn paint(
        &mut self,
        commands: &[PaintCommand],
        damage: &DamageRegion,
        _text: &mut T,
    ) -> Result<(), Self::Error> {
        self.last_commands = commands.to_vec();
        self.last_damage = damage.clone();
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        self.frames += 1;
        Ok(())
    }
}

pub trait FontRenderBackend {
    type Error;
}
