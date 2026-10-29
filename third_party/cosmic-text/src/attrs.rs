#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::hash::{Hash, Hasher};
use core::ops::Range;
use rangemap::RangeMap;
use smol_str::SmolStr;

use crate::{CacheKeyFlags, Metrics};

pub use fontdb::{Family, Stretch, Style, Weight};

#[derive(Clone, Copy, Debug, PartialOrd, Ord, Eq, Hash, PartialEq)]
pub struct Color(pub u32);

impl Color {
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 0xFF)
    }

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    #[inline]
    pub const fn as_rgba_tuple(self) -> (u8, u8, u8, u8) {
        (self.r(), self.g(), self.b(), self.a())
    }

    #[inline]
    pub const fn as_rgba(self) -> [u8; 4] {
        [self.r(), self.g(), self.b(), self.a()]
    }

    #[inline]
    pub const fn r(&self) -> u8 {
        ((self.0 & 0x00_FF_00_00) >> 16) as u8
    }

    #[inline]
    pub const fn g(&self) -> u8 {
        ((self.0 & 0x00_00_FF_00) >> 8) as u8
    }

    #[inline]
    pub const fn b(&self) -> u8 {
        (self.0 & 0x00_00_00_FF) as u8
    }

    #[inline]
    pub const fn a(&self) -> u8 {
        ((self.0 & 0xFF_00_00_00) >> 24) as u8
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum FamilyOwned {
    Name(SmolStr),
    Serif,
    SansSerif,
    Cursive,
    Fantasy,
    Monospace,
}

impl FamilyOwned {
    pub fn new(family: Family) -> Self {
        match family {
            Family::Name(name) => Self::Name(SmolStr::from(name)),
            Family::Serif => Self::Serif,
            Family::SansSerif => Self::SansSerif,
            Family::Cursive => Self::Cursive,
            Family::Fantasy => Self::Fantasy,
            Family::Monospace => Self::Monospace,
        }
    }

    pub fn as_family(&self) -> Family<'_> {
        match self {
            Self::Name(name) => Family::Name(name),
            Self::Serif => Family::Serif,
            Self::SansSerif => Family::SansSerif,
            Self::Cursive => Family::Cursive,
            Self::Fantasy => Family::Fantasy,
            Self::Monospace => Family::Monospace,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CacheMetrics {
    font_size_bits: u32,
    line_height_bits: u32,
}

impl From<Metrics> for CacheMetrics {
    fn from(metrics: Metrics) -> Self {
        Self {
            font_size_bits: metrics.font_size.to_bits(),
            line_height_bits: metrics.line_height.to_bits(),
        }
    }
}

impl From<CacheMetrics> for Metrics {
    fn from(metrics: CacheMetrics) -> Self {
        Self {
            font_size: f32::from_bits(metrics.font_size_bits),
            line_height: f32::from_bits(metrics.line_height_bits),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FeatureTag([u8; 4]);

impl FeatureTag {
    pub const fn new(tag: &[u8; 4]) -> Self {
        Self(*tag)
    }

    pub const KERNING: Self = Self::new(b"kern");
    pub const STANDARD_LIGATURES: Self = Self::new(b"liga");
    pub const CONTEXTUAL_LIGATURES: Self = Self::new(b"clig");
    pub const CONTEXTUAL_ALTERNATES: Self = Self::new(b"calt");
    pub const DISCRETIONARY_LIGATURES: Self = Self::new(b"dlig");
    pub const SMALL_CAPS: Self = Self::new(b"smcp");
    pub const ALL_SMALL_CAPS: Self = Self::new(b"c2sc");
    pub const STYLISTIC_SET_1: Self = Self::new(b"ss01");
    pub const STYLISTIC_SET_2: Self = Self::new(b"ss02");

    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Feature {
    pub tag: FeatureTag,
    pub value: u32,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct FontFeatures {
    pub features: Vec<Feature>,
}

impl FontFeatures {
    pub const fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }

    pub fn set(&mut self, tag: FeatureTag, value: u32) -> &mut Self {
        self.features.push(Feature { tag, value });
        self
    }

    pub fn enable(&mut self, tag: FeatureTag) -> &mut Self {
        self.set(tag, 1)
    }

    pub fn disable(&mut self, tag: FeatureTag) -> &mut Self {
        self.set(tag, 0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LetterSpacing(pub f32);

impl PartialEq for LetterSpacing {
    fn eq(&self, other: &Self) -> bool {
        if self.0.is_nan() {
            other.0.is_nan()
        } else {
            self.0 == other.0
        }
    }
}

impl Eq for LetterSpacing {}

impl Hash for LetterSpacing {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        const CANONICAL_NAN_BITS: u32 = 0x7fc0_0000;

        let bits = if self.0.is_nan() {
            CANONICAL_NAN_BITS
        } else {
            (self.0 + 0.0).to_bits()
        };

        bits.hash(hasher);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextDecoration {
    pub underline: UnderlineStyle,
    pub underline_color_opt: Option<Color>,
    pub strikethrough: bool,
    pub strikethrough_color_opt: Option<Color>,
    pub overline: bool,
    pub overline_color_opt: Option<Color>,
}

impl TextDecoration {
    pub const fn new() -> Self {
        Self {
            underline: UnderlineStyle::None,
            underline_color_opt: None,
            strikethrough: false,
            strikethrough_color_opt: None,
            overline: false,
            overline_color_opt: None,
        }
    }

    pub const fn has_decoration(&self) -> bool {
        !matches!(self.underline, UnderlineStyle::None) || self.strikethrough || self.overline
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DecorationMetrics {
    pub offset: f32,
    pub thickness: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlyphDecorationData {
    pub text_decoration: TextDecoration,
    pub underline_metrics: DecorationMetrics,
    pub strikethrough_metrics: DecorationMetrics,
    pub ascent: f32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Attrs<'a> {
    pub color_opt: Option<Color>,
    pub family: Family<'a>,
    pub stretch: Stretch,
    pub style: Style,
    pub weight: Weight,
    pub metadata: usize,
    pub cache_key_flags: CacheKeyFlags,
    pub metrics_opt: Option<CacheMetrics>,
    pub letter_spacing_opt: Option<LetterSpacing>,
    pub font_features: FontFeatures,
    pub text_decoration: TextDecoration,
}

impl<'a> Attrs<'a> {
    pub const fn new() -> Self {
        Self {
            color_opt: None,
            family: Family::SansSerif,
            stretch: Stretch::Normal,
            style: Style::Normal,
            weight: Weight::NORMAL,
            metadata: 0,
            cache_key_flags: CacheKeyFlags::empty(),
            metrics_opt: None,
            letter_spacing_opt: None,
            font_features: FontFeatures::new(),
            text_decoration: TextDecoration::new(),
        }
    }

    pub const fn color(mut self, color: Color) -> Self {
        self.color_opt = Some(color);
        self
    }

    pub const fn family(mut self, family: Family<'a>) -> Self {
        self.family = family;
        self
    }

    pub const fn stretch(mut self, stretch: Stretch) -> Self {
        self.stretch = stretch;
        self
    }

    pub const fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub const fn weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    pub const fn metadata(mut self, metadata: usize) -> Self {
        self.metadata = metadata;
        self
    }

    pub const fn cache_key_flags(mut self, cache_key_flags: CacheKeyFlags) -> Self {
        self.cache_key_flags = cache_key_flags;
        self
    }

    pub fn metrics(mut self, metrics: Metrics) -> Self {
        self.metrics_opt = Some(metrics.into());
        self
    }

    pub const fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.letter_spacing_opt = Some(LetterSpacing(letter_spacing));
        self
    }

    pub fn font_features(mut self, font_features: FontFeatures) -> Self {
        self.font_features = font_features;
        self
    }

    pub const fn underline(mut self, style: UnderlineStyle) -> Self {
        self.text_decoration.underline = style;
        self
    }

    pub const fn underline_color(mut self, color: Color) -> Self {
        self.text_decoration.underline_color_opt = Some(color);
        self
    }

    pub const fn strikethrough(mut self) -> Self {
        self.text_decoration.strikethrough = true;
        self
    }

    pub const fn strikethrough_color(mut self, color: Color) -> Self {
        self.text_decoration.strikethrough_color_opt = Some(color);
        self
    }

    pub const fn overline(mut self) -> Self {
        self.text_decoration.overline = true;
        self
    }

    pub const fn overline_color(mut self, color: Color) -> Self {
        self.text_decoration.overline_color_opt = Some(color);
        self
    }

    pub fn compatible(&self, other: &Self) -> bool {
        self.family == other.family
            && self.stretch == other.stretch
            && self.style == other.style
            && self.weight == other.weight
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FontMatchAttrs {
    family: FamilyOwned,
    stretch: Stretch,
    style: Style,
    weight: Weight,
}

impl<'a> From<&Attrs<'a>> for FontMatchAttrs {
    fn from(attrs: &Attrs<'a>) -> Self {
        Self {
            family: FamilyOwned::new(attrs.family),
            stretch: attrs.stretch,
            style: attrs.style,
            weight: attrs.weight,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AttrsOwned {
    pub color_opt: Option<Color>,
    pub family_owned: FamilyOwned,
    pub stretch: Stretch,
    pub style: Style,
    pub weight: Weight,
    pub metadata: usize,
    pub cache_key_flags: CacheKeyFlags,
    pub metrics_opt: Option<CacheMetrics>,
    pub letter_spacing_opt: Option<LetterSpacing>,
    pub font_features: FontFeatures,
    pub text_decoration: TextDecoration,
}

impl AttrsOwned {
    pub fn new(attrs: &Attrs) -> Self {
        Self {
            color_opt: attrs.color_opt,
            family_owned: FamilyOwned::new(attrs.family),
            stretch: attrs.stretch,
            style: attrs.style,
            weight: attrs.weight,
            metadata: attrs.metadata,
            cache_key_flags: attrs.cache_key_flags,
            metrics_opt: attrs.metrics_opt,
            letter_spacing_opt: attrs.letter_spacing_opt,
            font_features: attrs.font_features.clone(),
            text_decoration: attrs.text_decoration,
        }
    }

    pub fn as_attrs(&self) -> Attrs<'_> {
        Attrs {
            color_opt: self.color_opt,
            family: self.family_owned.as_family(),
            stretch: self.stretch,
            style: self.style,
            weight: self.weight,
            metadata: self.metadata,
            cache_key_flags: self.cache_key_flags,
            metrics_opt: self.metrics_opt,
            letter_spacing_opt: self.letter_spacing_opt,
            font_features: self.font_features.clone(),
            text_decoration: self.text_decoration,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AttrsList {
    defaults: AttrsOwned,
    pub(crate) spans: RangeMap<usize, AttrsOwned>,
}

impl AttrsList {
    pub fn new(defaults: &Attrs) -> Self {
        Self {
            defaults: AttrsOwned::new(defaults),
            spans: RangeMap::new(),
        }
    }

    pub fn defaults(&self) -> Attrs<'_> {
        self.defaults.as_attrs()
    }

    pub fn spans(&self) -> Vec<(&Range<usize>, &AttrsOwned)> {
        self.spans_iter().collect()
    }

    pub fn spans_iter(&self) -> impl Iterator<Item = (&Range<usize>, &AttrsOwned)> + '_ {
        self.spans.iter()
    }

    pub fn clear_spans(&mut self) {
        self.spans.clear();
    }

    pub fn add_span(&mut self, range: Range<usize>, attrs: &Attrs) {
        if range.is_empty() {
            return;
        }

        self.spans.insert(range, AttrsOwned::new(attrs));
    }

    pub fn get_span(&self, index: usize) -> Attrs<'_> {
        self.spans
            .get(&index)
            .map(|v| v.as_attrs())
            .unwrap_or(self.defaults.as_attrs())
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn split_off(&mut self, index: usize) -> Self {
        let mut new = Self::new(&self.defaults.as_attrs());
        let mut removes = Vec::new();

        for span in self.spans.iter() {
            if span.0.end <= index {
                continue;
            }

            if span.0.start >= index {
                removes.push((span.0.clone(), false));
            } else {
                removes.push((span.0.clone(), true));
            }
        }

        for (key, resize) in removes {
            let (range, attrs) = self
                .spans
                .get_key_value(&key.start)
                .map(|v| (v.0.clone(), v.1.clone()))
                .expect("attrs span not found");
            self.spans.remove(key);

            if resize {
                new.spans.insert(0..range.end - index, attrs.clone());
                self.spans.insert(range.start..index, attrs);
            } else {
                new.spans
                    .insert(range.start - index..range.end - index, attrs);
            }
        }
        new
    }

    pub(crate) fn reset(mut self, default: &Attrs) -> Self {
        self.defaults = AttrsOwned::new(default);
        self.spans.clear();
        self
    }
}
