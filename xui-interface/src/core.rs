use ordered_float::NotNan;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self::new(0.0, 0.0)
    }

    pub fn distance_to(&self, other: Point) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }

    pub fn translate(&self, translation: Translation) -> Self {
        Self::new(self.x + translation.x, self.y + translation.y)
    }

    pub fn scale(&self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Hash)]
pub enum Sizing {
    Fix(NotNan<f32>),
    Percent(NotNan<f32>),
    #[default]
    Hug,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size<P> {
    pub width: P,
    pub height: P,
}

impl Size<f32> {
    pub const ZERO: Self = Self::new((0.0), (0.0));

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl Size<Sizing> {
    pub const ZERO: Self = Self::new(Sizing::Hug, Sizing::Hug);

    pub const fn new(width: Sizing, height: Sizing) -> Self {
        Self { width, height }
    }

    pub fn fix(width: f32, height: f32) -> Self {
        Self {
            width: Sizing::Fix(NotNan::new(width).unwrap()),
            height: Sizing::Fix(NotNan::new(height).unwrap()),
        }
    }

    pub const fn hug() -> Self {
        Self {
            width: Sizing::Hug,
            height: Sizing::Hug,
        }
    }

    pub const fn fill() -> Self {
        Self {
            width: Sizing::Fill,
            height: Sizing::Fill,
        }
    }
}

impl<P: Copy> Size<P> {
    pub fn width(&self) -> P {
        self.width
    }

    pub fn height(&self) -> P {
        self.height
    }

    pub fn set_width(&mut self, width: P) {
        self.width = width;
    }

    pub fn set_height(&mut self, height: P) {
        self.height = height;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_origin_size(origin: Point, size: Size<f32>) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x <= self.x + self.width
            && point.y <= self.y + self.height
    }

    pub fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    pub fn union(self, other: Self) -> Self {
        if self.width <= 0.0 || self.height <= 0.0 {
            return other;
        }
        if other.width <= 0.0 || other.height <= 0.0 {
            return self;
        }

        let min_x = self.x.min(other.x);
        let min_y = self.y.min(other.y);
        let max_x = (self.x + self.width).max(other.x + other.width);
        let max_y = (self.y + self.height).max(other.y + other.height);
        Self::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    pub fn shrink(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - 2.0 * amount).max(0.0),
            (self.height - 2.0 * amount).max(0.0),
        )
    }

    pub fn scale(self, factor: f32) -> Self {
        Self::new(
            self.x * factor,
            self.y * factor,
            self.width * factor,
            self.height * factor,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const GRAY_100: Self = Self::rgb(0.94, 0.94, 0.94);
    pub const GRAY_300: Self = Self::rgb(0.72, 0.72, 0.72);
    pub const BLUE_500: Self = Self::rgb(0.18, 0.42, 0.88);

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeInsets {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl EdgeInsets {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(value: f32) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Translation {
    pub x: f32,
    pub y: f32,
}

impl Translation {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self::new(0.0, 0.0)
    }

    pub fn is_zero(&self) -> bool {
        self.x == 0.0 && self.y == 0.0
    }

    pub fn translate(&self, point: Point) -> Point {
        Point::new(point.x + self.x, point.y + self.y)
    }
}
