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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Damage {
    visual_rect: Rect,
    rect: Rect,
}

impl Damage {
    pub fn new(rect: Rect, visual_rect: Rect) -> Self {
        Self { rect, visual_rect }
    }

    pub fn full(size: Size<f32>) -> Self {
        Self {
            visual_rect: Rect::new(0., 0., size.width, size.height),
            rect: Rect::new(0., 0., size.width, size.height),
        }
    }

    pub fn rect(self) -> Rect {
        self.rect
    }

    pub fn visual_rect(self) -> Rect {
        self.visual_rect
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DamageRegion {
    rects: Vec<Damage>,
}

impl DamageRegion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, damage: Damage) {
        if damage.visual_rect.width > 0.0 && damage.visual_rect.height > 0.0 {
            self.rects.push(damage);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    pub fn take(&mut self) -> Self {
        Self {
            rects: std::mem::take(&mut self.rects),
        }
    }

    pub fn damages(&self) -> &[Damage] {
        &self.rects
    }

    pub fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.rects.iter().map(|damage| damage.rect)
    }

    pub fn visual_rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.rects.iter().map(|damage| damage.visual_rect)
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().map(|r| r.visual_rect).reduce(Rect::union)
    }

    pub fn intersects(&self, rect: Rect) -> bool {
        self.rects.iter().any(|d| d.visual_rect.intersects(rect))
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

#[derive(Debug, Clone)]
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
