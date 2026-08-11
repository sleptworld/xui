use ordered_float::NotNan;
use std::{
    hash::{Hash, Hasher},
    ops::{Add, Sub},
};

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

impl Sub<Point> for Point {
    type Output = Point;

    fn sub(self, rhs: Point) -> Self::Output {
        Point::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Add<Point> for Point {
    type Output = Point;

    fn add(self, rhs: Point) -> Self::Output {
        Point::new(self.x + rhs.x, self.y + rhs.y)
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

impl From<f32> for Sizing {
    fn from(value: f32) -> Self {
        Self::fix(value)
    }
}

impl From<NotNan<f32>> for Sizing {
    fn from(value: NotNan<f32>) -> Self {
        Self::Fix(value)
    }
}

impl From<u32> for Sizing {
    fn from(value: u32) -> Self {
        Self::Fix(NotNan::new(value as f32).unwrap())
    }
}

impl Sizing {
    pub fn fix(value: f32) -> Self {
        Self::Fix(NotNan::new(value).unwrap())
    }

    pub fn percent(value: f32) -> Self {
        Self::Percent(NotNan::new(value).unwrap())
    }

    pub const fn hug() -> Self {
        Self::Hug
    }

    pub const fn fill() -> Self {
        Self::Fill
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size<P = f32> {
    pub width: P,
    pub height: P,
}

impl Size<f32> {
    pub const ZERO: Self = Self::new(0.0, 0.0);
}

impl<T> Size<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

impl Size<Sizing> {
    pub const ZERO: Self = Self::new(Sizing::Hug, Sizing::Hug);

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

impl Size<f32> {
    pub fn aspect_ratio(&self) -> f32 {
        self.width / self.height
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

    pub fn expand(self, amount: f32) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.width + amount * 2.0,
            self.height + amount * 2.0,
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

    pub fn translate(self, translation: impl Into<Translation>) -> Self {
        let translation = translation.into();
        Self::new(
            self.x + translation.x,
            self.y + translation.y,
            self.width,
            self.height,
        )
    }

    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn min_x(&self) -> f32 {
        self.x
    }

    pub fn min_y(&self) -> f32 {
        self.y
    }

    pub fn max_x(&self) -> f32 {
        self.x + self.width
    }

    pub fn max_y(&self) -> f32 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

fn hash_f32_canonical<H: Hasher>(x: f32, state: &mut H) {
    debug_assert!(!x.is_nan());

    let bits = if x == 0.0 {
        0.0f32.to_bits()
    } else {
        x.to_bits()
    };

    bits.hash(state);
}

impl Hash for Color {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32_canonical(self.r, state);
        hash_f32_canonical(self.g, state);
        hash_f32_canonical(self.b, state);
        hash_f32_canonical(self.a, state);
    }
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

    pub fn hex(hex: &str) -> Self {
        let hex = hex.trim_start_matches('#');

        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).unwrap();
            let g = u8::from_str_radix(&hex[1..2], 16).unwrap();
            let b = u8::from_str_radix(&hex[2..3], 16).unwrap();
            Self::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
        } else if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap();
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap();
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap();
            Self::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
        } else {
            panic!("Invalid hex color: {}", hex);
        }
    }

    pub fn alpha(mut self, alpha: f32) -> Self {
        self.a = alpha;
        self
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Hash)]
pub struct EdgeInsets {
    pub left: NotNan<f32>,
    pub right: NotNan<f32>,
    pub top: NotNan<f32>,
    pub bottom: NotNan<f32>,
}

impl EdgeInsets {
    pub fn zero() -> Self {
        Self::all(0.0)
    }

    pub fn new(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        let left = NotNan::new(left).unwrap();
        let right = NotNan::new(right).unwrap();
        let top = NotNan::new(top).unwrap();
        let bottom = NotNan::new(bottom).unwrap();
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    pub fn all(value: f32) -> Self {
        let value = NotNan::new(value).unwrap();
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }

    pub fn symmetric(horizontal: f32, vertical: f32) -> Self {
        let horizontal = NotNan::new(horizontal).unwrap();
        let vertical = NotNan::new(vertical).unwrap();
        Self {
            left: horizontal,
            right: horizontal,
            top: vertical,
            bottom: vertical,
        }
    }

    #[inline(always)]
    pub fn left(&self) -> f32 {
        self.left.into_inner()
    }
    #[inline(always)]
    pub fn right(&self) -> f32 {
        self.right.into_inner()
    }

    #[inline(always)]
    pub fn top(&self) -> f32 {
        self.top.into_inner()
    }

    #[inline(always)]
    pub fn bottom(&self) -> f32 {
        self.bottom.into_inner()
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

impl From<Point> for Translation {
    fn from(point: Point) -> Self {
        Self::new(point.x, point.y)
    }
}
