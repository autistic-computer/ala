bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    #[repr(transparent)]
    pub struct CacheKeyFlags: u32 {
        const FAKE_ITALIC = 1;
        const DISABLE_HINTING = 2;
        const PIXEL_FONT = 4;
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CacheKey {
    pub font_id: fontdb::ID,
    pub glyph_id: u16,
    pub font_size_bits: u32,
    pub x_bin: SubpixelBin,
    pub y_bin: SubpixelBin,
    pub font_weight: fontdb::Weight,
    pub flags: CacheKeyFlags,
}

impl CacheKey {
    pub fn new(
        font_id: fontdb::ID,
        glyph_id: u16,
        font_size: f32,
        pos: (f32, f32),
        weight: fontdb::Weight,
        flags: CacheKeyFlags,
    ) -> (Self, i32, i32) {
        let (x, x_bin) = SubpixelBin::new(pos.0);
        let (y, y_bin) = SubpixelBin::new(pos.1);
        (
            Self {
                font_id,
                glyph_id,
                font_size_bits: font_size.to_bits(),
                x_bin,
                y_bin,
                flags,
                font_weight: weight,
            },
            x,
            y,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SubpixelBin {
    Zero,
    One,
    Two,
    Three,
}

impl SubpixelBin {
    pub fn new(pos: f32) -> (i32, Self) {
        let trunc = pos as i32;
        let fract = pos - trunc as f32;

        if pos.is_sign_negative() {
            if fract > -0.125 {
                (trunc, Self::Zero)
            } else if fract > -0.375 {
                (trunc - 1, Self::Three)
            } else if fract > -0.625 {
                (trunc - 1, Self::Two)
            } else if fract > -0.875 {
                (trunc - 1, Self::One)
            } else {
                (trunc - 1, Self::Zero)
            }
        } else {
            #[allow(clippy::collapsible_else_if)]
            if fract < 0.125 {
                (trunc, Self::Zero)
            } else if fract < 0.375 {
                (trunc, Self::One)
            } else if fract < 0.625 {
                (trunc, Self::Two)
            } else if fract < 0.875 {
                (trunc, Self::Three)
            } else {
                (trunc + 1, Self::Zero)
            }
        }
    }

    pub const fn as_float(&self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::One => 0.25,
            Self::Two => 0.5,
            Self::Three => 0.75,
        }
    }
}

#[test]
fn test_subpixel_bins() {
    assert_eq!(SubpixelBin::new(0.0), (0, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(0.124), (0, SubpixelBin::Zero));

    assert_eq!(SubpixelBin::new(0.125), (0, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(0.25), (0, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(0.374), (0, SubpixelBin::One));

    assert_eq!(SubpixelBin::new(0.375), (0, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(0.5), (0, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(0.624), (0, SubpixelBin::Two));

    assert_eq!(SubpixelBin::new(0.625), (0, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(0.75), (0, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(0.874), (0, SubpixelBin::Three));

    assert_eq!(SubpixelBin::new(0.875), (1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(0.999), (1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(1.0), (1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(1.124), (1, SubpixelBin::Zero));

    assert_eq!(SubpixelBin::new(-0.0), (0, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-0.124), (0, SubpixelBin::Zero));

    assert_eq!(SubpixelBin::new(-0.125), (-1, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(-0.25), (-1, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(-0.374), (-1, SubpixelBin::Three));

    assert_eq!(SubpixelBin::new(-0.375), (-1, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(-0.5), (-1, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(-0.624), (-1, SubpixelBin::Two));

    assert_eq!(SubpixelBin::new(-0.625), (-1, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(-0.75), (-1, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(-0.874), (-1, SubpixelBin::One));

    assert_eq!(SubpixelBin::new(-0.875), (-1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-0.999), (-1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-1.0), (-1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-1.124), (-1, SubpixelBin::Zero));
}
