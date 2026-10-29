use crate::refineable::Refineable;
use anyhow::{Context as _, anyhow};
use core::fmt::Debug;
use derive_more::{Add, AddAssign, Div, DivAssign, Mul, Neg, Sub, SubAssign};
use schemars::{JsonSchema, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::borrow::Cow;
use std::ops::{AddAssign, Range};
use std::{
    cmp::{self, PartialOrd},
    fmt::{self, Display},
    hash::Hash,
    ops::{Add, Div, Mul, MulAssign, Neg, Sub},
};
use taffy::prelude::{TaffyGridLine, TaffyGridSpan};

use crate::{App, DisplayId};

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    pub fn invert(self) -> Self {
        match self {
            Axis::Vertical => Axis::Horizontal,
            Axis::Horizontal => Axis::Vertical,
        }
    }
}

pub trait Along {
    type Unit;

    fn along(&self, axis: Axis) -> Self::Unit;

    fn apply_along(&self, axis: Axis, f: impl FnOnce(Self::Unit) -> Self::Unit) -> Self;
}

#[derive(
    Refineable,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    Hash,
    Neg,
)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Point<T: Clone + Debug + Default + PartialEq> {
    pub x: T,
    pub y: T,
}

pub const fn point<T: Clone + Debug + Default + PartialEq>(x: T, y: T) -> Point<T> {
    Point { x, y }
}

impl<T: Clone + Debug + Default + PartialEq> Point<T> {
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn map<U: Clone + Debug + Default + PartialEq>(&self, f: impl Fn(T) -> U) -> Point<U> {
        Point {
            x: f(self.x.clone()),
            y: f(self.y.clone()),
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq> Along for Point<T> {
    type Unit = T;

    fn along(&self, axis: Axis) -> T {
        match axis {
            Axis::Horizontal => self.x.clone(),
            Axis::Vertical => self.y.clone(),
        }
    }

    fn apply_along(&self, axis: Axis, f: impl FnOnce(T) -> T) -> Point<T> {
        match axis {
            Axis::Horizontal => Point {
                x: f(self.x.clone()),
                y: self.y.clone(),
            },
            Axis::Vertical => Point {
                x: self.x.clone(),
                y: f(self.y.clone()),
            },
        }
    }
}

impl Point<Pixels> {
    pub fn scale(&self, factor: f32) -> Point<ScaledPixels> {
        Point {
            x: self.x.scale(factor),
            y: self.y.scale(factor),
        }
    }

    pub fn magnitude(&self) -> f64 {
        ((self.x.0.powi(2) + self.y.0.powi(2)) as f64).sqrt()
    }
}

impl<T> Point<T>
where
    T: Sub<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    pub fn relative_to(&self, origin: &Point<T>) -> Point<T> {
        point(
            self.x.clone() - origin.x.clone(),
            self.y.clone() - origin.y.clone(),
        )
    }
}

impl<T, Rhs> Mul<Rhs> for Point<T>
where
    T: Mul<Rhs, Output = T> + Clone + Debug + Default + PartialEq,
    Rhs: Clone + Debug,
{
    type Output = Point<T>;

    fn mul(self, rhs: Rhs) -> Self::Output {
        Point {
            x: self.x * rhs.clone(),
            y: self.y * rhs,
        }
    }
}

impl<T, S> MulAssign<S> for Point<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.x = self.x.clone() * rhs.clone();
        self.y = self.y.clone() * rhs;
    }
}

impl<T, S> Div<S> for Point<T>
where
    T: Div<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    type Output = Self;

    fn div(self, rhs: S) -> Self::Output {
        Self {
            x: self.x / rhs.clone(),
            y: self.y / rhs,
        }
    }
}

impl<T> Point<T>
where
    T: PartialOrd + Clone + Debug + Default + PartialEq,
{
    pub fn max(&self, other: &Self) -> Self {
        Point {
            x: if self.x > other.x {
                self.x.clone()
            } else {
                other.x.clone()
            },
            y: if self.y > other.y {
                self.y.clone()
            } else {
                other.y.clone()
            },
        }
    }

    pub fn min(&self, other: &Self) -> Self {
        Point {
            x: if self.x <= other.x {
                self.x.clone()
            } else {
                other.x.clone()
            },
            y: if self.y <= other.y {
                self.y.clone()
            } else {
                other.y.clone()
            },
        }
    }

    pub fn clamp(&self, min: &Self, max: &Self) -> Self {
        self.max(min).min(max)
    }
}

impl<T: Clone + Debug + Default + PartialEq> Clone for Point<T> {
    fn clone(&self) -> Self {
        Self {
            x: self.x.clone(),
            y: self.y.clone(),
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq + Display> Display for Point<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

#[derive(
    Add, Clone, Copy, Default, Deserialize, Div, Hash, Neg, PartialEq, Refineable, Serialize, Sub,
)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Size<T: Clone + Debug + Default + PartialEq> {
    pub width: T,
    pub height: T,
}

impl<T: Clone + Debug + Default + PartialEq> Size<T> {
    pub fn new(width: T, height: T) -> Self {
        size(width, height)
    }
}

pub const fn size<T>(width: T, height: T) -> Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    Size { width, height }
}

impl<T> Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    pub fn map<U>(&self, f: impl Fn(T) -> U) -> Size<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Size {
            width: f(self.width.clone()),
            height: f(self.height.clone()),
        }
    }
}

impl<T> Size<T>
where
    T: Clone + Debug + Default + PartialEq + Half,
{
    pub fn center(&self) -> Point<T> {
        Point {
            x: self.width.half(),
            y: self.height.half(),
        }
    }
}

impl Size<Pixels> {
    pub fn scale(&self, factor: f32) -> Size<ScaledPixels> {
        Size {
            width: self.width.scale(factor),
            height: self.height.scale(factor),
        }
    }
}

impl<T> Along for Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    type Unit = T;

    fn along(&self, axis: Axis) -> T {
        match axis {
            Axis::Horizontal => self.width.clone(),
            Axis::Vertical => self.height.clone(),
        }
    }

    fn apply_along(&self, axis: Axis, f: impl FnOnce(T) -> T) -> Self {
        match axis {
            Axis::Horizontal => Size {
                width: f(self.width.clone()),
                height: self.height.clone(),
            },
            Axis::Vertical => Size {
                width: self.width.clone(),
                height: f(self.height.clone()),
            },
        }
    }
}

impl<T> Size<T>
where
    T: PartialOrd + Clone + Debug + Default + PartialEq,
{
    pub fn max(&self, other: &Self) -> Self {
        Size {
            width: if self.width >= other.width {
                self.width.clone()
            } else {
                other.width.clone()
            },
            height: if self.height >= other.height {
                self.height.clone()
            } else {
                other.height.clone()
            },
        }
    }

    pub fn min(&self, other: &Self) -> Self {
        Size {
            width: if self.width >= other.width {
                other.width.clone()
            } else {
                self.width.clone()
            },
            height: if self.height >= other.height {
                other.height.clone()
            } else {
                self.height.clone()
            },
        }
    }
}

impl<T, Rhs> Mul<Rhs> for Size<T>
where
    T: Mul<Rhs, Output = Rhs> + Clone + Debug + Default + PartialEq,
    Rhs: Clone + Debug + Default + PartialEq,
{
    type Output = Size<Rhs>;

    fn mul(self, rhs: Rhs) -> Self::Output {
        Size {
            width: self.width * rhs.clone(),
            height: self.height * rhs,
        }
    }
}

impl<T, S> MulAssign<S> for Size<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.width = self.width.clone() * rhs.clone();
        self.height = self.height.clone() * rhs;
    }
}

impl<T> Eq for Size<T> where T: Eq + Clone + Debug + Default + PartialEq {}

impl<T> Debug for Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Size {{ {:?} × {:?} }}", self.width, self.height)
    }
}

impl<T: Clone + Debug + Default + PartialEq + Display> Display for Size<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} × {}", self.width, self.height)
    }
}

impl<T: Clone + Debug + Default + PartialEq> From<Point<T>> for Size<T> {
    fn from(point: Point<T>) -> Self {
        Self {
            width: point.x,
            height: point.y,
        }
    }
}

impl From<Size<Pixels>> for Size<DefiniteLength> {
    fn from(size: Size<Pixels>) -> Self {
        Size {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

impl From<Size<Pixels>> for Size<AbsoluteLength> {
    fn from(size: Size<Pixels>) -> Self {
        Size {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

impl Size<Length> {
    pub fn full() -> Self {
        Self {
            width: relative(1.).into(),
            height: relative(1.).into(),
        }
    }
}

impl Size<Length> {
    pub fn auto() -> Self {
        Self {
            width: Length::Auto,
            height: Length::Auto,
        }
    }
}

#[derive(Refineable, Copy, Clone, Default, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
#[refineable(Debug)]
#[repr(C)]
pub struct Bounds<T: Clone + Debug + Default + PartialEq> {
    pub origin: Point<T>,
    pub size: Size<T>,
}

pub fn bounds<T: Clone + Debug + Default + PartialEq>(
    origin: Point<T>,
    size: Size<T>,
) -> Bounds<T> {
    Bounds { origin, size }
}

impl Bounds<Pixels> {
    pub fn centered(display_id: Option<DisplayId>, size: Size<Pixels>, cx: &App) -> Self {
        let display = display_id
            .and_then(|id| cx.find_display(id))
            .or_else(|| cx.primary_display());

        display
            .map(|display| {
                let visible_bounds = display.visible_bounds();
                Bounds::centered_at(visible_bounds.center(), size.min(&visible_bounds.size))
            })
            .unwrap_or_else(|| Bounds {
                origin: point(px(0.), px(0.)),
                size,
            })
    }

    pub fn maximized(display_id: Option<DisplayId>, cx: &App) -> Self {
        let display = display_id
            .and_then(|id| cx.find_display(id))
            .or_else(|| cx.primary_display());

        display
            .map(|display| display.bounds())
            .unwrap_or_else(|| Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(1024.), px(768.)),
            })
    }
}

impl<T> Bounds<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    pub fn new(origin: Point<T>, size: Size<T>) -> Self {
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<Output = T> + Clone + Debug + Default + PartialEq,
{
    pub fn from_corners(top_left: Point<T>, bottom_right: Point<T>) -> Self {
        let origin = Point {
            x: top_left.x.clone(),
            y: top_left.y.clone(),
        };
        let size = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    pub fn from_anchor_and_size(corner: Anchor, origin: Point<T>, size: Size<T>) -> Bounds<T> {
        let origin = match corner {
            Anchor::TopLeft => origin,
            Anchor::TopRight => Point {
                x: origin.x - size.width.clone(),
                y: origin.y,
            },
            Anchor::BottomLeft => Point {
                x: origin.x,
                y: origin.y - size.height.clone(),
            },
            Anchor::BottomRight => Point {
                x: origin.x - size.width.clone(),
                y: origin.y - size.height.clone(),
            },
            Anchor::TopCenter => Point {
                x: origin.x - size.width.half(),
                y: origin.y,
            },
            Anchor::BottomCenter => Point {
                x: origin.x - size.width.half(),
                y: origin.y - size.height.clone(),
            },
            Anchor::LeftCenter => Point {
                x: origin.x,
                y: origin.y - size.height.half(),
            },
            Anchor::RightCenter => Point {
                x: origin.x - size.width.clone(),
                y: origin.y - size.height.half(),
            },
        };

        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    pub fn centered_at(center: Point<T>, size: Size<T>) -> Self {
        let origin = Point {
            x: center.x - size.width.half(),
            y: center.y - size.height.half(),
        };
        Self::new(origin, size)
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    pub fn top_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.half(),
            y: self.origin.y.clone(),
        }
    }

    pub fn bottom_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.half(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }

    pub fn left_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone(),
            y: self.origin.y.clone() + self.size.height.half(),
        }
    }

    pub fn right_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone() + self.size.height.half(),
        }
    }
}

impl<T> Bounds<T>
where
    T: PartialOrd + Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    pub fn intersects(&self, other: &Bounds<T>) -> bool {
        let my_lower_right = self.bottom_right();
        let their_lower_right = other.bottom_right();

        self.origin.x < their_lower_right.x
            && my_lower_right.x > other.origin.x
            && self.origin.y < their_lower_right.y
            && my_lower_right.y > other.origin.y
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    pub fn center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone().half(),
            y: self.origin.y.clone() + self.size.height.clone().half(),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    pub fn half_perimeter(&self) -> T {
        self.size.width.clone() + self.size.height.clone()
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<Output = T> + Clone + Debug + Default + PartialEq,
{
    #[must_use]
    pub fn dilate(&self, amount: T) -> Bounds<T> {
        let double_amount = amount.clone() + amount.clone();
        Bounds {
            origin: self.origin.clone() - point(amount.clone(), amount),
            size: self.size.clone() + size(double_amount.clone(), double_amount),
        }
    }

    #[must_use]
    pub fn extend(&self, amount: Edges<T>) -> Bounds<T> {
        Bounds {
            origin: self.origin.clone() - point(amount.left.clone(), amount.top.clone()),
            size: self.size.clone()
                + size(
                    amount.left.clone() + amount.right.clone(),
                    amount.top.clone() + amount.bottom,
                ),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T>
        + Sub<T, Output = T>
        + Neg<Output = T>
        + Clone
        + Debug
        + Default
        + PartialEq,
{
    pub fn inset(&self, amount: T) -> Self {
        self.dilate(-amount)
    }
}

impl<T: PartialOrd + Add<T, Output = T> + Sub<Output = T> + Clone + Debug + Default + PartialEq>
    Bounds<T>
{
    pub fn intersect(&self, other: &Self) -> Self {
        let upper_left = self.origin.max(&other.origin);
        let bottom_right = self
            .bottom_right()
            .min(&other.bottom_right())
            .max(&upper_left);
        Self::from_corners(upper_left, bottom_right)
    }

    pub fn union(&self, other: &Self) -> Self {
        let top_left = self.origin.min(&other.origin);
        let bottom_right = self.bottom_right().max(&other.bottom_right());
        Bounds::from_corners(top_left, bottom_right)
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    pub fn space_within(&self, outer: &Self) -> Edges<T> {
        Edges {
            top: self.top() - outer.top(),
            right: outer.right() - self.right(),
            bottom: outer.bottom() - self.bottom(),
            left: self.left() - outer.left(),
        }
    }
}

impl<T, Rhs> Mul<Rhs> for Bounds<T>
where
    T: Mul<Rhs, Output = Rhs> + Clone + Debug + Default + PartialEq,
    Point<T>: Mul<Rhs, Output = Point<Rhs>>,
    Rhs: Clone + Debug + Default + PartialEq,
{
    type Output = Bounds<Rhs>;

    fn mul(self, rhs: Rhs) -> Self::Output {
        Bounds {
            origin: self.origin * rhs.clone(),
            size: self.size * rhs,
        }
    }
}

impl<T, S> MulAssign<S> for Bounds<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.origin *= rhs.clone();
        self.size *= rhs;
    }
}

impl<T, S> Div<S> for Bounds<T>
where
    Size<T>: Div<S, Output = Size<T>>,
    T: Div<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    type Output = Self;

    fn div(self, rhs: S) -> Self {
        Self {
            origin: self.origin / rhs.clone(),
            size: self.size / rhs,
        }
    }
}

impl<T> Add<Point<T>> for Bounds<T>
where
    T: Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn add(self, rhs: Point<T>) -> Self {
        Self {
            origin: self.origin + rhs,
            size: self.size,
        }
    }
}

impl<T> Sub<Point<T>> for Bounds<T>
where
    T: Sub<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn sub(self, rhs: Point<T>) -> Self {
        Self {
            origin: self.origin - rhs,
            size: self.size,
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq> From<Size<T>> for Point<T> {
    fn from(size: Size<T>) -> Self {
        Self {
            x: size.width,
            y: size.height,
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    pub fn top(&self) -> T {
        self.origin.y.clone()
    }

    pub fn bottom(&self) -> T {
        self.origin.y.clone() + self.size.height.clone()
    }

    pub fn left(&self) -> T {
        self.origin.x.clone()
    }

    pub fn right(&self) -> T {
        self.origin.x.clone() + self.size.width.clone()
    }

    pub fn top_right(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone(),
        }
    }

    pub fn bottom_right(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }

    pub fn bottom_left(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    pub fn corner(&self, corner: Anchor) -> Point<T> {
        match corner {
            Anchor::TopLeft => self.origin.clone(),
            Anchor::TopRight => self.top_right(),
            Anchor::BottomLeft => self.bottom_left(),
            Anchor::BottomRight => self.bottom_right(),
            Anchor::TopCenter => self.top_center(),
            Anchor::BottomCenter => self.bottom_center(),
            Anchor::LeftCenter => self.left_center(),
            Anchor::RightCenter => self.right_center(),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + PartialOrd + Clone + Debug + Default + PartialEq,
{
    pub fn contains(&self, point: &Point<T>) -> bool {
        point.x >= self.origin.x
            && point.x < self.origin.x.clone() + self.size.width.clone()
            && point.y >= self.origin.y
            && point.y < self.origin.y.clone() + self.size.height.clone()
    }

    pub fn is_contained_within(&self, other: &Self) -> bool {
        other.contains(&self.origin) && other.contains(&self.bottom_right())
    }

    pub fn map<U>(&self, f: impl Fn(T) -> U) -> Bounds<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Bounds {
            origin: self.origin.map(&f),
            size: self.size.map(f),
        }
    }

    pub fn map_origin(self, f: impl Fn(T) -> T) -> Bounds<T> {
        Bounds {
            origin: self.origin.map(f),
            size: self.size,
        }
    }

    pub fn map_size(self, f: impl Fn(T) -> T) -> Bounds<T> {
        Bounds {
            origin: self.origin,
            size: self.size.map(f),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<T, Output = T> + PartialOrd + Clone + Debug + Default + PartialEq,
{
    pub fn localize(&self, point: &Point<T>) -> Option<Point<T>> {
        self.contains(point)
            .then(|| point.relative_to(&self.origin))
    }
}

impl<T: PartialOrd + Clone + Debug + Default + PartialEq> Bounds<T> {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.width <= T::default() || self.size.height <= T::default()
    }
}

impl<T: Clone + Debug + Default + PartialEq + Display + Add<T, Output = T>> Display for Bounds<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} - {} (size {})",
            self.origin,
            self.bottom_right(),
            self.size
        )
    }
}

impl Size<DevicePixels> {
    pub fn to_pixels(self, scale_factor: f32) -> Size<Pixels> {
        size(
            px(self.width.0 as f32 / scale_factor),
            px(self.height.0 as f32 / scale_factor),
        )
    }
}

impl Size<Pixels> {
    pub fn to_device_pixels(self, scale_factor: f32) -> Size<DevicePixels> {
        size(
            DevicePixels((self.width.0 * scale_factor).round() as i32),
            DevicePixels((self.height.0 * scale_factor).round() as i32),
        )
    }
}

impl Bounds<Pixels> {
    pub fn scale(&self, factor: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: self.origin.scale(factor),
            size: self.size.scale(factor),
        }
    }

    pub fn to_device_pixels(self, factor: f32) -> Bounds<DevicePixels> {
        Bounds {
            origin: point(
                DevicePixels((self.origin.x.0 * factor).round() as i32),
                DevicePixels((self.origin.y.0 * factor).round() as i32),
            ),
            size: self.size.to_device_pixels(factor),
        }
    }
}

impl Bounds<DevicePixels> {
    pub fn to_pixels(self, scale_factor: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(
                px(self.origin.x.0 as f32 / scale_factor),
                px(self.origin.y.0 as f32 / scale_factor),
            ),
            size: self.size.to_pixels(scale_factor),
        }
    }
}

#[derive(Refineable, Clone, Default, Debug, Eq, PartialEq)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Edges<T: Clone + Debug + Default + PartialEq> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T> Mul for Edges<T>
where
    T: Mul<Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            top: self.top.clone() * rhs.top,
            right: self.right.clone() * rhs.right,
            bottom: self.bottom.clone() * rhs.bottom,
            left: self.left * rhs.left,
        }
    }
}

impl<T, S> MulAssign<S> for Edges<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.top = self.top.clone() * rhs.clone();
        self.right = self.right.clone() * rhs.clone();
        self.bottom = self.bottom.clone() * rhs.clone();
        self.left = self.left.clone() * rhs;
    }
}

impl<T: Clone + Debug + Default + PartialEq + Copy> Copy for Edges<T> {}

impl<T: Clone + Debug + Default + PartialEq> Edges<T> {
    pub fn all(value: T) -> Self {
        Self {
            top: value.clone(),
            right: value.clone(),
            bottom: value.clone(),
            left: value,
        }
    }

    pub fn map<U>(&self, f: impl Fn(&T) -> U) -> Edges<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Edges {
            top: f(&self.top),
            right: f(&self.right),
            bottom: f(&self.bottom),
            left: f(&self.left),
        }
    }

    pub fn any<F: Fn(&T) -> bool>(&self, predicate: F) -> bool {
        predicate(&self.top)
            || predicate(&self.right)
            || predicate(&self.bottom)
            || predicate(&self.left)
    }
}

impl Edges<Length> {
    pub fn auto() -> Self {
        Self {
            top: Length::Auto,
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::Auto,
        }
    }

    pub fn zero() -> Self {
        Self {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        }
    }
}

impl Edges<DefiniteLength> {
    pub fn zero() -> Self {
        Self {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        }
    }

    pub fn to_pixels(self, parent_size: Size<AbsoluteLength>, rem_size: Pixels) -> Edges<Pixels> {
        Edges {
            top: self.top.to_pixels(parent_size.height, rem_size),
            right: self.right.to_pixels(parent_size.width, rem_size),
            bottom: self.bottom.to_pixels(parent_size.height, rem_size),
            left: self.left.to_pixels(parent_size.width, rem_size),
        }
    }
}

impl Edges<AbsoluteLength> {
    pub fn zero() -> Self {
        Self {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        }
    }

    pub fn to_pixels(self, rem_size: Pixels) -> Edges<Pixels> {
        Edges {
            top: self.top.to_pixels(rem_size),
            right: self.right.to_pixels(rem_size),
            bottom: self.bottom.to_pixels(rem_size),
            left: self.left.to_pixels(rem_size),
        }
    }
}

impl Edges<Pixels> {
    pub fn scale(&self, factor: f32) -> Edges<ScaledPixels> {
        Edges {
            top: self.top.scale(factor),
            right: self.right.scale(factor),
            bottom: self.bottom.scale(factor),
            left: self.left.scale(factor),
        }
    }

    pub fn max(&self) -> Pixels {
        self.top.max(self.right).max(self.bottom).max(self.left)
    }
}

impl From<f32> for Edges<Pixels> {
    fn from(val: f32) -> Self {
        let val: Pixels = val.into();
        val.into()
    }
}

impl From<Pixels> for Edges<Pixels> {
    fn from(val: Pixels) -> Self {
        Edges {
            top: val,
            right: val,
            bottom: val,
            left: val,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

impl Anchor {
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Anchor::TopLeft => Anchor::BottomRight,
            Anchor::TopRight => Anchor::BottomLeft,
            Anchor::BottomLeft => Anchor::TopRight,
            Anchor::BottomRight => Anchor::TopLeft,
            Anchor::TopCenter => Anchor::BottomCenter,
            Anchor::BottomCenter => Anchor::TopCenter,
            Anchor::LeftCenter => Anchor::RightCenter,
            Anchor::RightCenter => Anchor::LeftCenter,
        }
    }

    #[must_use]
    pub fn other_side_along(self, axis: Axis) -> Self {
        match axis {
            Axis::Vertical => match self {
                Anchor::TopLeft => Anchor::BottomLeft,
                Anchor::TopRight => Anchor::BottomRight,
                Anchor::BottomLeft => Anchor::TopLeft,
                Anchor::BottomRight => Anchor::TopRight,
                Anchor::TopCenter => Anchor::BottomCenter,
                Anchor::BottomCenter => Anchor::TopCenter,
                Anchor::LeftCenter => Anchor::LeftCenter,
                Anchor::RightCenter => Anchor::RightCenter,
            },
            Axis::Horizontal => match self {
                Anchor::TopLeft => Anchor::TopRight,
                Anchor::TopRight => Anchor::TopLeft,
                Anchor::BottomLeft => Anchor::BottomRight,
                Anchor::BottomRight => Anchor::BottomLeft,
                Anchor::TopCenter => Anchor::TopCenter,
                Anchor::BottomCenter => Anchor::BottomCenter,
                Anchor::LeftCenter => Anchor::RightCenter,
                Anchor::RightCenter => Anchor::LeftCenter,
            },
        }
    }

    #[inline]
    pub fn is_center(&self) -> bool {
        matches!(
            self,
            Self::TopCenter | Self::BottomCenter | Self::LeftCenter | Self::RightCenter
        )
    }

    #[inline]
    pub fn is_bottom(&self) -> bool {
        matches!(
            self,
            Self::BottomCenter | Self::BottomLeft | Self::BottomRight
        )
    }
}

#[derive(Refineable, Clone, Default, Debug, Eq, PartialEq)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Corners<T: Clone + Debug + Default + PartialEq> {
    pub top_left: T,
    pub top_right: T,
    pub bottom_right: T,
    pub bottom_left: T,
}

impl<T> Corners<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    pub fn all(value: T) -> Self {
        Self {
            top_left: value.clone(),
            top_right: value.clone(),
            bottom_right: value.clone(),
            bottom_left: value,
        }
    }

    #[must_use]
    pub fn corner(&self, corner: Anchor) -> T {
        match corner {
            Anchor::TopLeft => self.top_left.clone(),
            Anchor::TopRight => self.top_right.clone(),
            Anchor::BottomLeft => self.bottom_left.clone(),
            Anchor::BottomRight => self.bottom_right.clone(),
            Anchor::TopCenter => (self.top_left.clone() + self.top_right.clone()).half(),
            Anchor::BottomCenter => (self.bottom_left.clone() + self.bottom_right.clone()).half(),
            Anchor::LeftCenter => (self.top_left.clone() + self.bottom_left.clone()).half(),
            Anchor::RightCenter => (self.top_right.clone() + self.bottom_right.clone()).half(),
        }
    }
}

impl Corners<AbsoluteLength> {
    pub fn to_pixels(self, rem_size: Pixels) -> Corners<Pixels> {
        Corners {
            top_left: self.top_left.to_pixels(rem_size),
            top_right: self.top_right.to_pixels(rem_size),
            bottom_right: self.bottom_right.to_pixels(rem_size),
            bottom_left: self.bottom_left.to_pixels(rem_size),
        }
    }
}

impl Corners<Pixels> {
    #[must_use]
    pub fn scale(&self, factor: f32) -> Corners<ScaledPixels> {
        Corners {
            top_left: self.top_left.scale(factor),
            top_right: self.top_right.scale(factor),
            bottom_right: self.bottom_right.scale(factor),
            bottom_left: self.bottom_left.scale(factor),
        }
    }

    #[must_use]
    pub fn max(&self) -> Pixels {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

impl<T: Div<f32, Output = T> + Ord + Clone + Debug + Default + PartialEq> Corners<T> {
    #[must_use]
    pub fn resolve_radii_for_quad_size(self, size: Size<Pixels>, fraction: f32) -> Corners<Pixels>
    where
        T: Into<Pixels>,
    {
        let shorter_side = cmp::min(size.width, size.height);
        let full = shorter_side / 2.;
        let cap = shorter_side * fraction;
        let resolve = |radius: T| {
            let radius: Pixels = radius.into();
            if radius >= FULL_CORNER_RADIUS {
                full
            } else if cap > Pixels::ZERO {
                cap * (radius / cap).tanh()
            } else {
                Pixels::ZERO
            }
        };

        Corners {
            top_left: resolve(self.top_left),
            top_right: resolve(self.top_right),
            bottom_right: resolve(self.bottom_right),
            bottom_left: resolve(self.bottom_left),
        }
    }

    #[must_use]
    pub fn clamp_radii_for_quad_size(self, size: Size<T>) -> Corners<T> {
        let max = cmp::min(size.width, size.height) / 2.;
        Corners {
            top_left: cmp::min(self.top_left, max.clone()),
            top_right: cmp::min(self.top_right, max.clone()),
            bottom_right: cmp::min(self.bottom_right, max.clone()),
            bottom_left: cmp::min(self.bottom_left, max),
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq> Corners<T> {
    #[must_use]
    pub fn map<U>(&self, f: impl Fn(&T) -> U) -> Corners<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Corners {
            top_left: f(&self.top_left),
            top_right: f(&self.top_right),
            bottom_right: f(&self.bottom_right),
            bottom_left: f(&self.bottom_left),
        }
    }
}

impl<T> Mul for Corners<T>
where
    T: Mul<Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            top_left: self.top_left.clone() * rhs.top_left,
            top_right: self.top_right.clone() * rhs.top_right,
            bottom_right: self.bottom_right.clone() * rhs.bottom_right,
            bottom_left: self.bottom_left * rhs.bottom_left,
        }
    }
}

impl<T, S> MulAssign<S> for Corners<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.top_left = self.top_left.clone() * rhs.clone();
        self.top_right = self.top_right.clone() * rhs.clone();
        self.bottom_right = self.bottom_right.clone() * rhs.clone();
        self.bottom_left = self.bottom_left.clone() * rhs;
    }
}

impl<T> Copy for Corners<T> where T: Copy + Clone + Debug + Default + PartialEq {}

impl From<f32> for Corners<Pixels> {
    fn from(val: f32) -> Self {
        Corners {
            top_left: val.into(),
            top_right: val.into(),
            bottom_right: val.into(),
            bottom_left: val.into(),
        }
    }
}

impl From<Pixels> for Corners<Pixels> {
    fn from(val: Pixels) -> Self {
        Corners {
            top_left: val,
            top_right: val,
            bottom_right: val,
            bottom_left: val,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Neg,
    Div,
    DivAssign,
    PartialEq,
    Serialize,
    Deserialize,
    Debug,
)]
#[repr(transparent)]
pub struct Radians(pub f32);

pub fn radians(value: f32) -> Radians {
    Radians(value)
}

#[derive(
    Clone,
    Copy,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Neg,
    Div,
    DivAssign,
    PartialEq,
    Serialize,
    Deserialize,
    Debug,
)]
#[repr(transparent)]
pub struct Percentage(pub f32);

pub fn percentage(value: f32) -> Percentage {
    debug_assert!(
        (0.0..=1.0).contains(&value),
        "Percentage must be between 0 and 1"
    );
    Percentage(value)
}

impl From<Percentage> for Radians {
    fn from(value: Percentage) -> Self {
        radians(value.0 * std::f32::consts::PI * 2.0)
    }
}

#[derive(
    Clone,
    Copy,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Neg,
    Div,
    DivAssign,
    PartialEq,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[repr(transparent)]
pub struct Pixels(pub(crate) f32);

impl Div for Pixels {
    type Output = f32;

    fn div(self, rhs: Self) -> Self::Output {
        self.0 / rhs.0
    }
}

impl std::ops::DivAssign for Pixels {
    fn div_assign(&mut self, rhs: Self) {
        *self = Self(self.0 / rhs.0);
    }
}

impl std::ops::RemAssign for Pixels {
    fn rem_assign(&mut self, rhs: Self) {
        self.0 %= rhs.0;
    }
}

impl std::ops::Rem for Pixels {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self {
        Self(self.0 % rhs.0)
    }
}

impl Mul<f32> for Pixels {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * rhs)
    }
}

impl Mul<Pixels> for f32 {
    type Output = Pixels;

    fn mul(self, rhs: Pixels) -> Self::Output {
        rhs * self
    }
}

impl Mul<usize> for Pixels {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        self * (rhs as f32)
    }
}

impl Mul<Pixels> for usize {
    type Output = Pixels;

    fn mul(self, rhs: Pixels) -> Pixels {
        rhs * self
    }
}

impl MulAssign<f32> for Pixels {
    fn mul_assign(&mut self, rhs: f32) {
        self.0 *= rhs;
    }
}

impl Display for Pixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}px", self.0)
    }
}

impl Debug for Pixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl std::iter::Sum for Pixels {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + b)
    }
}

impl<'a> std::iter::Sum<&'a Pixels> for Pixels {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + *b)
    }
}

impl TryFrom<&'_ str> for Pixels {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        value
            .strip_suffix("px")
            .context("expected 'px' suffix")
            .and_then(|number| Ok(number.parse()?))
            .map(Self)
    }
}

impl Pixels {
    pub const ZERO: Pixels = Pixels(0.0);
    pub const MAX: Pixels = Pixels(f32::MAX);
    pub const MIN: Pixels = Pixels(f32::MIN);

    pub const fn as_f32(self) -> f32 {
        self.0
    }

    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    pub fn ceil(&self) -> Self {
        Self(self.0.ceil())
    }

    #[must_use]
    pub fn scale(&self, factor: f32) -> ScaledPixels {
        ScaledPixels(self.0 * factor)
    }

    pub fn pow(&self, exponent: f32) -> Self {
        Self(self.0.powf(exponent))
    }

    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    pub fn signum(&self) -> f32 {
        self.0.signum()
    }

    pub fn to_f64(self) -> f64 {
        self.0 as f64
    }
}

impl Eq for Pixels {}

impl PartialOrd for Pixels {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pixels {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl std::hash::Hash for Pixels {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl From<f64> for Pixels {
    fn from(pixels: f64) -> Self {
        Pixels(pixels as f32)
    }
}

impl From<f32> for Pixels {
    fn from(pixels: f32) -> Self {
        Pixels(pixels)
    }
}

impl From<Pixels> for f32 {
    fn from(pixels: Pixels) -> Self {
        pixels.0
    }
}

impl From<&Pixels> for f32 {
    fn from(pixels: &Pixels) -> Self {
        pixels.0
    }
}

impl From<Pixels> for f64 {
    fn from(pixels: Pixels) -> Self {
        pixels.0 as f64
    }
}

impl From<Pixels> for u32 {
    fn from(pixels: Pixels) -> Self {
        pixels.0 as u32
    }
}

impl From<&Pixels> for u32 {
    fn from(pixels: &Pixels) -> Self {
        pixels.0 as u32
    }
}

impl From<u32> for Pixels {
    fn from(pixels: u32) -> Self {
        Pixels(pixels as f32)
    }
}

impl From<Pixels> for usize {
    fn from(pixels: Pixels) -> Self {
        pixels.0 as usize
    }
}

impl From<usize> for Pixels {
    fn from(pixels: usize) -> Self {
        Pixels(pixels as f32)
    }
}

#[derive(
    Add,
    AddAssign,
    Clone,
    Copy,
    Default,
    Div,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Sub,
    SubAssign,
    Serialize,
    Deserialize,
)]
#[repr(transparent)]
pub struct DevicePixels(pub i32);

impl DevicePixels {
    pub fn to_bytes(self, bytes_per_pixel: u8) -> u32 {
        self.0 as u32 * bytes_per_pixel as u32
    }
}

impl fmt::Debug for DevicePixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} px (device)", self.0)
    }
}

impl From<DevicePixels> for i32 {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0
    }
}

impl From<i32> for DevicePixels {
    fn from(device_pixels: i32) -> Self {
        DevicePixels(device_pixels)
    }
}

impl From<u32> for DevicePixels {
    fn from(device_pixels: u32) -> Self {
        DevicePixels(device_pixels as i32)
    }
}

impl From<DevicePixels> for u32 {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0 as u32
    }
}

impl From<DevicePixels> for u64 {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0 as u64
    }
}

impl From<u64> for DevicePixels {
    fn from(device_pixels: u64) -> Self {
        DevicePixels(device_pixels as i32)
    }
}

impl From<DevicePixels> for usize {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0 as usize
    }
}

impl From<usize> for DevicePixels {
    fn from(device_pixels: usize) -> Self {
        DevicePixels(device_pixels as i32)
    }
}

#[derive(Clone, Copy, Default, Add, AddAssign, Sub, SubAssign, Div, DivAssign, PartialEq)]
#[repr(transparent)]
pub struct ScaledPixels(pub f32);

impl ScaledPixels {
    pub fn as_f32(self) -> f32 {
        self.0
    }

    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    pub fn ceil(&self) -> Self {
        Self(self.0.ceil())
    }
}

impl Eq for ScaledPixels {}

impl PartialOrd for ScaledPixels {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScaledPixels {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Debug for ScaledPixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}px (scaled)", self.0)
    }
}

impl From<ScaledPixels> for DevicePixels {
    fn from(scaled: ScaledPixels) -> Self {
        DevicePixels(scaled.0.ceil() as i32)
    }
}

impl From<DevicePixels> for ScaledPixels {
    fn from(device: DevicePixels) -> Self {
        ScaledPixels(device.0 as f32)
    }
}

impl From<ScaledPixels> for f64 {
    fn from(scaled_pixels: ScaledPixels) -> Self {
        scaled_pixels.0 as f64
    }
}

impl From<ScaledPixels> for u32 {
    fn from(pixels: ScaledPixels) -> Self {
        pixels.0 as u32
    }
}

impl From<f32> for ScaledPixels {
    fn from(pixels: f32) -> Self {
        Self(pixels)
    }
}

impl Div for ScaledPixels {
    type Output = f32;

    fn div(self, rhs: Self) -> Self::Output {
        self.0 / rhs.0
    }
}

impl std::ops::DivAssign for ScaledPixels {
    fn div_assign(&mut self, rhs: Self) {
        *self = Self(self.0 / rhs.0);
    }
}

impl std::ops::RemAssign for ScaledPixels {
    fn rem_assign(&mut self, rhs: Self) {
        self.0 %= rhs.0;
    }
}

impl std::ops::Rem for ScaledPixels {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self {
        Self(self.0 % rhs.0)
    }
}

impl Mul<f32> for ScaledPixels {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * rhs)
    }
}

impl Mul<ScaledPixels> for f32 {
    type Output = ScaledPixels;

    fn mul(self, rhs: ScaledPixels) -> Self::Output {
        rhs * self
    }
}

impl Mul<usize> for ScaledPixels {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        self * (rhs as f32)
    }
}

impl Mul<ScaledPixels> for usize {
    type Output = ScaledPixels;

    fn mul(self, rhs: ScaledPixels) -> ScaledPixels {
        rhs * self
    }
}

impl MulAssign<f32> for ScaledPixels {
    fn mul_assign(&mut self, rhs: f32) {
        self.0 *= rhs;
    }
}

#[derive(Clone, Copy, Default, Add, Sub, Mul, Div, Neg, PartialEq)]
pub struct Rems(pub f32);

impl Rems {
    pub const ZERO: Self = Self(0.0);
    pub fn to_pixels(self, rem_size: Pixels) -> Pixels {
        self * rem_size
    }
    pub fn from_pixels(length: Pixels, window: &ala::Window) -> Self {
        Self(length / window.rem_size())
    }
}

impl Mul<Pixels> for Rems {
    type Output = Pixels;

    fn mul(self, other: Pixels) -> Pixels {
        Pixels(self.0 * other.0)
    }
}

impl AddAssign<Rems> for Rems {
    fn add_assign(&mut self, rhs: Rems) {
        self.0 += rhs.0
    }
}

impl Display for Rems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}rem", self.0)
    }
}

impl Debug for Rems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl TryFrom<&'_ str> for Rems {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        value
            .strip_suffix("rem")
            .context("expected 'rem' suffix")
            .and_then(|number| Ok(number.parse()?))
            .map(Self)
    }
}

#[derive(Clone, Copy, Neg, PartialEq)]
pub enum AbsoluteLength {
    Pixels(Pixels),
    Rems(Rems),
}

impl AbsoluteLength {
    pub fn is_zero(&self) -> bool {
        match self {
            AbsoluteLength::Pixels(px) => px.0 == 0.0,
            AbsoluteLength::Rems(rems) => rems.0 == 0.0,
        }
    }
}

impl From<Pixels> for AbsoluteLength {
    fn from(pixels: Pixels) -> Self {
        AbsoluteLength::Pixels(pixels)
    }
}

impl From<Rems> for AbsoluteLength {
    fn from(rems: Rems) -> Self {
        AbsoluteLength::Rems(rems)
    }
}

impl AbsoluteLength {
    pub fn to_pixels(self, rem_size: Pixels) -> Pixels {
        match self {
            AbsoluteLength::Pixels(pixels) => pixels,
            AbsoluteLength::Rems(rems) => rems.to_pixels(rem_size),
        }
    }

    pub fn to_rems(self, rem_size: Pixels) -> Rems {
        match self {
            AbsoluteLength::Pixels(pixels) => Rems(pixels.0 / rem_size.0),
            AbsoluteLength::Rems(rems) => rems,
        }
    }
}

impl Default for AbsoluteLength {
    fn default() -> Self {
        px(0.).into()
    }
}

impl Display for AbsoluteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pixels(pixels) => write!(f, "{pixels}"),
            Self::Rems(rems) => write!(f, "{rems}"),
        }
    }
}

impl Debug for AbsoluteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

const EXPECTED_ABSOLUTE_LENGTH: &str = "number with 'px' or 'rem' suffix";

impl TryFrom<&'_ str> for AbsoluteLength {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        if let Ok(pixels) = value.try_into() {
            Ok(Self::Pixels(pixels))
        } else if let Ok(rems) = value.try_into() {
            Ok(Self::Rems(rems))
        } else {
            Err(anyhow!(
                "invalid AbsoluteLength '{value}', expected {EXPECTED_ABSOLUTE_LENGTH}"
            ))
        }
    }
}

impl JsonSchema for AbsoluteLength {
    fn schema_name() -> Cow<'static, str> {
        "AbsoluteLength".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^-?\d+(\.\d+)?(px|rem)$"
        })
    }
}

impl<'de> Deserialize<'de> for AbsoluteLength {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl de::Visitor<'_> for StringVisitor {
            type Value = AbsoluteLength;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{EXPECTED_ABSOLUTE_LENGTH}")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                AbsoluteLength::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(StringVisitor)
    }
}

impl Serialize for AbsoluteLength {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

#[derive(Clone, Copy, Neg, PartialEq)]
pub enum DefiniteLength {
    Absolute(AbsoluteLength),
    Fraction(f32),
}

impl DefiniteLength {
    pub fn to_pixels(self, base_size: AbsoluteLength, rem_size: Pixels) -> Pixels {
        match self {
            DefiniteLength::Absolute(size) => size.to_pixels(rem_size),
            DefiniteLength::Fraction(fraction) => match base_size {
                AbsoluteLength::Pixels(px) => px * fraction,
                AbsoluteLength::Rems(rems) => rems * rem_size * fraction,
            },
        }
    }
}

impl Debug for DefiniteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for DefiniteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefiniteLength::Absolute(length) => write!(f, "{length}"),
            DefiniteLength::Fraction(fraction) => write!(f, "{}%", (fraction * 100.0) as i32),
        }
    }
}

const EXPECTED_DEFINITE_LENGTH: &str = "expected number with 'px', 'rem', or '%' suffix";

impl TryFrom<&'_ str> for DefiniteLength {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        if let Some(percentage) = value.strip_suffix('%') {
            let fraction: f32 = percentage.parse::<f32>().with_context(|| {
                format!("invalid DefiniteLength '{value}', expected {EXPECTED_DEFINITE_LENGTH}")
            })?;
            Ok(DefiniteLength::Fraction(fraction / 100.0))
        } else if let Ok(absolute_length) = value.try_into() {
            Ok(DefiniteLength::Absolute(absolute_length))
        } else {
            Err(anyhow!(
                "invalid DefiniteLength '{value}', expected {EXPECTED_DEFINITE_LENGTH}"
            ))
        }
    }
}

impl JsonSchema for DefiniteLength {
    fn schema_name() -> Cow<'static, str> {
        "DefiniteLength".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^-?\d+(\.\d+)?(px|rem|%)$"
        })
    }
}

impl<'de> Deserialize<'de> for DefiniteLength {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl de::Visitor<'_> for StringVisitor {
            type Value = DefiniteLength;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{EXPECTED_DEFINITE_LENGTH}")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                DefiniteLength::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(StringVisitor)
    }
}

impl Serialize for DefiniteLength {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

impl From<Pixels> for DefiniteLength {
    fn from(pixels: Pixels) -> Self {
        Self::Absolute(pixels.into())
    }
}

impl From<Rems> for DefiniteLength {
    fn from(rems: Rems) -> Self {
        Self::Absolute(rems.into())
    }
}

impl From<AbsoluteLength> for DefiniteLength {
    fn from(length: AbsoluteLength) -> Self {
        Self::Absolute(length)
    }
}

impl Default for DefiniteLength {
    fn default() -> Self {
        Self::Absolute(AbsoluteLength::default())
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Length {
    Definite(DefiniteLength),
    Auto,
}

impl Debug for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Length::Definite(definite_length) => write!(f, "{}", definite_length),
            Length::Auto => write!(f, "auto"),
        }
    }
}

const EXPECTED_LENGTH: &str = "expected 'auto' or number with 'px', 'rem', or '%' suffix";

impl TryFrom<&'_ str> for Length {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        if value == "auto" {
            Ok(Length::Auto)
        } else if let Ok(definite_length) = value.try_into() {
            Ok(Length::Definite(definite_length))
        } else {
            Err(anyhow!(
                "invalid Length '{value}', expected {EXPECTED_LENGTH}"
            ))
        }
    }
}

impl JsonSchema for Length {
    fn schema_name() -> Cow<'static, str> {
        "Length".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^(auto|-?\d+(\.\d+)?(px|rem|%))$"
        })
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl de::Visitor<'_> for StringVisitor {
            type Value = Length;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{EXPECTED_LENGTH}")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Length::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(StringVisitor)
    }
}

impl Serialize for Length {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

pub const fn relative(fraction: f32) -> DefiniteLength {
    DefiniteLength::Fraction(fraction)
}

pub const fn phi() -> DefiniteLength {
    relative(std::f32::consts::GOLDEN_RATIO)
}

pub const fn rems(rems: f32) -> Rems {
    Rems(rems)
}

pub const fn px(pixels: f32) -> Pixels {
    Pixels(pixels)
}

pub const FULL_CORNER_RADIUS: Pixels = px(9999.);

pub const fn auto() -> Length {
    Length::Auto
}

impl From<Pixels> for Length {
    fn from(pixels: Pixels) -> Self {
        Self::Definite(pixels.into())
    }
}

impl From<Rems> for Length {
    fn from(rems: Rems) -> Self {
        Self::Definite(rems.into())
    }
}

impl From<DefiniteLength> for Length {
    fn from(length: DefiniteLength) -> Self {
        Self::Definite(length)
    }
}

impl From<AbsoluteLength> for Length {
    fn from(length: AbsoluteLength) -> Self {
        Self::Definite(length.into())
    }
}

impl Default for Length {
    fn default() -> Self {
        Self::Definite(DefiniteLength::default())
    }
}

impl From<()> for Length {
    fn from(_: ()) -> Self {
        Self::Definite(DefiniteLength::default())
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub struct GridLocation {
    pub row: Range<GridPlacement>,
    pub column: Range<GridPlacement>,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub enum GridPlacement {
    Line(i16),
    Span(u16),
    #[default]
    Auto,
}

impl From<GridPlacement> for taffy::GridPlacement {
    fn from(placement: GridPlacement) -> Self {
        match placement {
            GridPlacement::Line(index) => taffy::GridPlacement::from_line_index(index),
            GridPlacement::Span(span) => taffy::GridPlacement::from_span(span),
            GridPlacement::Auto => taffy::GridPlacement::Auto,
        }
    }
}

pub trait Half {
    fn half(&self) -> Self;
}

impl Half for i32 {
    fn half(&self) -> Self {
        self / 2
    }
}

impl Half for f32 {
    fn half(&self) -> Self {
        self / 2.
    }
}

impl Half for DevicePixels {
    fn half(&self) -> Self {
        Self(self.0 / 2)
    }
}

impl Half for ScaledPixels {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

impl Half for Pixels {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

impl Half for Rems {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

pub trait IsZero {
    fn is_zero(&self) -> bool;
}

impl IsZero for DevicePixels {
    fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl IsZero for ScaledPixels {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for Pixels {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for Rems {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for AbsoluteLength {
    fn is_zero(&self) -> bool {
        match self {
            AbsoluteLength::Pixels(pixels) => pixels.is_zero(),
            AbsoluteLength::Rems(rems) => rems.is_zero(),
        }
    }
}

impl IsZero for DefiniteLength {
    fn is_zero(&self) -> bool {
        match self {
            DefiniteLength::Absolute(length) => length.is_zero(),
            DefiniteLength::Fraction(fraction) => *fraction == 0.,
        }
    }
}

impl IsZero for Length {
    fn is_zero(&self) -> bool {
        match self {
            Length::Definite(length) => length.is_zero(),
            Length::Auto => false,
        }
    }
}

impl<T: IsZero + Clone + Debug + Default + PartialEq> IsZero for Point<T> {
    fn is_zero(&self) -> bool {
        self.x.is_zero() && self.y.is_zero()
    }
}

impl<T> IsZero for Size<T>
where
    T: IsZero + Clone + Debug + Default + PartialEq,
{
    fn is_zero(&self) -> bool {
        self.width.is_zero() || self.height.is_zero()
    }
}

impl<T: IsZero + Clone + Debug + Default + PartialEq> IsZero for Bounds<T> {
    fn is_zero(&self) -> bool {
        self.size.is_zero()
    }
}

impl<T> IsZero for Corners<T>
where
    T: IsZero + Clone + Debug + Default + PartialEq,
{
    fn is_zero(&self) -> bool {
        self.top_left.is_zero()
            && self.top_right.is_zero()
            && self.bottom_right.is_zero()
            && self.bottom_left.is_zero()
    }
}

#[cfg(test)]
mod adaptive_corner_tests {
    use super::{Corners, FULL_CORNER_RADIUS, px, size};

    const FRACTION: f32 = 0.45;

    fn resolved(radius: f32, width: f32, height: f32) -> Corners<crate::Pixels> {
        Corners::all(px(radius)).resolve_radii_for_quad_size(size(px(width), px(height)), FRACTION)
    }

    #[test]
    fn an_ordinary_radius_never_reaches_a_full_round() {
        for radius in 0..=256 {
            for side in [12.0, 16.0, 18.0, 24.0, 28.0, 30.0, 36.0, 40.0, 120.0] {
                let corner = resolved(radius as f32, side * 3.0, side);
                assert!(
                    corner.top_left < px(side / 2.0),
                    "radius {radius} on a {side}px side gave {:?}, which is a pill",
                    corner.top_left
                );
            }
        }
    }

    #[test]
    fn a_declared_full_corner_stays_full() {
        let corners = Corners::all(FULL_CORNER_RADIUS)
            .resolve_radii_for_quad_size(size(px(84.0), px(28.0)), FRACTION);
        assert_eq!(corners.top_left, px(14.0));
    }

    #[test]
    fn a_small_radius_is_near_enough_the_request() {
        for radius in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0] {
            let corner = resolved(radius, 84.0, 28.0).top_left;
            assert!(
                (corner - px(radius)).abs() < px(1.0),
                "radius {radius} resolved to {corner:?}, too far from the request"
            );
        }
    }

    #[test]
    fn the_radius_never_stops_responding() {
        for side in [16.0, 24.0, 28.0, 36.0] {
            let mut previous = resolved(0.0, side * 3.0, side).top_left;
            for radius in 1..=256 {
                let corner = resolved(radius as f32, side * 3.0, side).top_left;
                if radius <= 24 {
                    assert!(
                        corner > previous,
                        "a {side}px control stopped responding at radius {radius}"
                    );
                } else {
                    assert!(corner >= previous, "a {side}px control went backwards");
                }
                previous = corner;
            }
        }
    }

    #[test]
    fn each_corner_resolves_on_its_own() {
        let corners = Corners {
            top_left: FULL_CORNER_RADIUS,
            top_right: FULL_CORNER_RADIUS,
            bottom_right: px(40.0),
            bottom_left: px(40.0),
        }
        .resolve_radii_for_quad_size(size(px(84.0), px(28.0)), FRACTION);
        assert_eq!(corners.top_left, px(14.0));
        assert!(corners.bottom_right < px(14.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounds_intersects() {
        let bounds1 = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
        };
        let bounds2 = Bounds {
            origin: Point { x: 4.0, y: 4.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
        };
        let bounds3 = Bounds {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
        };

        assert!(bounds1.intersects(&bounds2));

        assert!(!bounds1.intersects(&bounds3));

        assert!(bounds1.intersects(&bounds1));
    }
}
