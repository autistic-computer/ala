use crate::{
    self as ala, AbsoluteLength, AlignContent, AlignItems, AlignSelf, BorderStyle,
    CornerRadiusMode, CursorStyle, DefiniteLength, Display, Fill, FlexDirection, FlexWrap, Font,
    FontFeatures, FontStyle, FontWeight, GridPlacement, GridTemplate, GridTemplateMinSize, Hsla,
    JustifyContent, Length, Pixels, SharedString, StrikethroughStyle, StyleRefinement, TextAlign,
    TextOverflow, TextStyleRefinement, UnderlineStyle, WhiteSpace, px, relative, rems,
};
pub use ala_macros::{
    border_style_methods, box_shadow_style_methods, cursor_style_methods, margin_style_methods,
    overflow_style_methods, padding_style_methods, position_style_methods,
    visibility_style_methods,
};
const ELLIPSIS: SharedString = SharedString::new_static("…");

#[cfg_attr(
    all(any(feature = "inspector", debug_assertions), not(rust_analyzer)),
    ala_macros::derive_inspector_reflection
)]
pub trait Styled: Sized {
    fn style(&mut self) -> &mut StyleRefinement;

    ala_macros::style_helpers!();
    ala_macros::visibility_style_methods!();
    ala_macros::margin_style_methods!();
    ala_macros::padding_style_methods!();
    ala_macros::position_style_methods!();
    ala_macros::overflow_style_methods!();
    ala_macros::cursor_style_methods!();
    ala_macros::border_style_methods!();
    ala_macros::box_shadow_style_methods!();

    #[doc = "Set this element's corner radius policy without changing its children."]
    fn corner_radius_mode(mut self, mode: CornerRadiusMode) -> Self {
        self.style().corner_radius_mode = Some(mode);
        self
    }

    #[doc = "Override corner smoothing for this element's fill, border and shadows only."]
    fn corner_smoothing(mut self, smoothing: f32) -> Self {
        self.style().corner_smoothing = Some(smoothing);
        self
    }

    fn block(mut self) -> Self {
        self.style().display = Some(Display::Block);
        self
    }

    fn flex(mut self) -> Self {
        self.style().display = Some(Display::Flex);
        self
    }

    fn grid(mut self) -> Self {
        self.style().display = Some(Display::Grid);
        self
    }

    fn hidden(mut self) -> Self {
        self.style().display = Some(Display::None);
        self
    }

    fn scrollbar_width(mut self, width: impl Into<AbsoluteLength>) -> Self {
        self.style().scrollbar_width = Some(width.into());
        self
    }

    fn whitespace_normal(mut self) -> Self {
        self.text_style().white_space = Some(WhiteSpace::Normal);
        self
    }

    fn whitespace_nowrap(mut self) -> Self {
        self.text_style().white_space = Some(WhiteSpace::Nowrap);
        self
    }

    fn text_ellipsis(mut self) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::Truncate(ELLIPSIS));
        self
    }

    fn text_ellipsis_start(mut self) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::TruncateStart(ELLIPSIS));
        self
    }

    fn text_ellipsis_middle(mut self) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::TruncateMiddle(ELLIPSIS));
        self
    }

    fn text_fade(mut self, width: Pixels) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::Fade(width));
        self
    }

    fn text_overflow(mut self, overflow: TextOverflow) -> Self {
        self.text_style().text_overflow = Some(overflow);
        self
    }

    fn text_align(mut self, align: TextAlign) -> Self {
        self.text_style().text_align = Some(align);
        self
    }

    fn text_left(mut self) -> Self {
        self.text_align(TextAlign::Left)
    }

    fn text_center(mut self) -> Self {
        self.text_align(TextAlign::Center)
    }

    fn text_right(mut self) -> Self {
        self.text_align(TextAlign::Right)
    }

    fn truncate(mut self) -> Self {
        self.overflow_hidden().whitespace_nowrap().text_ellipsis()
    }

    fn line_clamp(mut self, lines: usize) -> Self {
        let mut text_style = self.text_style();
        text_style.line_clamp = Some(lines);
        self.overflow_hidden()
    }

    fn flex_col(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::Column);
        self
    }

    fn flex_col_reverse(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::ColumnReverse);
        self
    }

    fn flex_row(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::Row);
        self
    }

    fn flex_row_reverse(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::RowReverse);
        self
    }

    fn flex_1(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(relative(0.).into());
        self
    }

    fn flex_auto(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(Length::Auto);
        self
    }

    fn flex_initial(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(Length::Auto);
        self
    }

    fn flex_none(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self.style().flex_shrink = Some(0.);
        self.style().flex_basis = Some(Length::Auto);
        self
    }

    fn flex_basis(mut self, basis: impl Into<Length>) -> Self {
        self.style().flex_basis = Some(basis.into());
        self
    }

    fn flex_grow(mut self, grow: f32) -> Self {
        self.style().flex_grow = Some(grow);
        self
    }

    fn flex_grow_0(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self
    }

    fn flex_grow_1(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self
    }

    fn flex_shrink(mut self, shrink: f32) -> Self {
        self.style().flex_shrink = Some(shrink);
        self
    }

    fn flex_shrink_0(mut self) -> Self {
        self.style().flex_shrink = Some(0.);
        self
    }

    fn flex_shrink_1(mut self) -> Self {
        self.style().flex_shrink = Some(1.);
        self
    }

    fn flex_wrap(mut self) -> Self {
        self.style().flex_wrap = Some(FlexWrap::Wrap);
        self
    }

    fn flex_wrap_reverse(mut self) -> Self {
        self.style().flex_wrap = Some(FlexWrap::WrapReverse);
        self
    }

    fn flex_nowrap(mut self) -> Self {
        self.style().flex_wrap = Some(FlexWrap::NoWrap);
        self
    }

    fn items_start(mut self) -> Self {
        self.style().align_items = Some(AlignItems::FlexStart);
        self
    }

    fn items_end(mut self) -> Self {
        self.style().align_items = Some(AlignItems::FlexEnd);
        self
    }

    fn items_center(mut self) -> Self {
        self.style().align_items = Some(AlignItems::Center);
        self
    }

    fn items_baseline(mut self) -> Self {
        self.style().align_items = Some(AlignItems::Baseline);
        self
    }

    fn items_stretch(mut self) -> Self {
        self.style().align_items = Some(AlignItems::Stretch);
        self
    }

    fn self_start(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Start);
        self
    }

    fn self_end(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::End);
        self
    }

    fn self_flex_start(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::FlexStart);
        self
    }

    fn self_flex_end(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::FlexEnd);
        self
    }

    fn self_center(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Center);
        self
    }

    fn self_baseline(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Baseline);
        self
    }

    fn self_stretch(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Stretch);
        self
    }

    fn justify_start(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::Start);
        self
    }

    fn justify_end(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::End);
        self
    }

    fn justify_center(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::Center);
        self
    }

    fn justify_between(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::SpaceBetween);
        self
    }

    fn justify_around(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::SpaceAround);
        self
    }

    fn justify_evenly(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::SpaceEvenly);
        self
    }

    fn content_normal(mut self) -> Self {
        self.style().align_content = None;
        self
    }

    fn content_center(mut self) -> Self {
        self.style().align_content = Some(AlignContent::Center);
        self
    }

    fn content_start(mut self) -> Self {
        self.style().align_content = Some(AlignContent::FlexStart);
        self
    }

    fn content_end(mut self) -> Self {
        self.style().align_content = Some(AlignContent::FlexEnd);
        self
    }

    fn content_between(mut self) -> Self {
        self.style().align_content = Some(AlignContent::SpaceBetween);
        self
    }

    fn content_around(mut self) -> Self {
        self.style().align_content = Some(AlignContent::SpaceAround);
        self
    }

    fn content_evenly(mut self) -> Self {
        self.style().align_content = Some(AlignContent::SpaceEvenly);
        self
    }

    fn content_stretch(mut self) -> Self {
        self.style().align_content = Some(AlignContent::Stretch);
        self
    }

    fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.style().aspect_ratio = Some(ratio);
        self
    }

    fn aspect_square(mut self) -> Self {
        self.style().aspect_ratio = Some(1.0);
        self
    }

    fn bg<F>(mut self, fill: F) -> Self
    where
        F: Into<Fill>,
        Self: Sized,
    {
        self.style().background = Some(fill.into());
        self
    }

    fn border_dashed(mut self) -> Self {
        self.style().border_style = Some(BorderStyle::Dashed);
        self
    }

    fn text_style(&mut self) -> &mut TextStyleRefinement {
        let style: &mut StyleRefinement = self.style();
        &mut style.text
    }

    fn text_color(mut self, color: impl Into<Hsla>) -> Self {
        self.text_style().color = Some(color.into());
        self
    }

    fn font_weight(mut self, weight: FontWeight) -> Self {
        self.text_style().font_weight = Some(weight);
        self
    }

    fn text_bg(mut self, bg: impl Into<Hsla>) -> Self {
        self.text_style().background_color = Some(bg.into());
        self
    }

    fn text_size(mut self, size: impl Into<AbsoluteLength>) -> Self {
        self.text_style().font_size = Some(size.into());
        self
    }

    fn text_xs(mut self) -> Self {
        self.text_style().font_size = Some(rems(0.75).into());
        self
    }

    fn text_sm(mut self) -> Self {
        self.text_style().font_size = Some(rems(0.875).into());
        self
    }

    fn text_base(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.0).into());
        self
    }

    fn text_lg(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.125).into());
        self
    }

    fn text_xl(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.25).into());
        self
    }

    fn text_2xl(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.5).into());
        self
    }

    fn text_3xl(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.875).into());
        self
    }

    fn italic(mut self) -> Self {
        self.text_style().font_style = Some(FontStyle::Italic);
        self
    }

    fn not_italic(mut self) -> Self {
        self.text_style().font_style = Some(FontStyle::Normal);
        self
    }

    fn underline(mut self) -> Self {
        let style = self.text_style();
        style.underline = Some(UnderlineStyle {
            thickness: px(1.),
            ..Default::default()
        });
        self
    }

    fn line_through(mut self) -> Self {
        let style = self.text_style();
        style.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            ..Default::default()
        });
        self
    }

    fn text_decoration_none(mut self) -> Self {
        self.text_style().underline = None;
        self
    }

    fn text_decoration_color(mut self, color: impl Into<Hsla>) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.color = Some(color.into());
        self
    }

    fn text_decoration_solid(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.wavy = false;
        self
    }

    fn text_decoration_wavy(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.wavy = true;
        self
    }

    fn text_decoration_0(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(0.);
        self
    }

    fn text_decoration_1(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(1.);
        self
    }

    fn text_decoration_2(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(2.);
        self
    }

    fn text_decoration_4(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(4.);
        self
    }

    fn text_decoration_8(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(8.);
        self
    }

    fn font_family(mut self, family_name: impl Into<SharedString>) -> Self {
        self.text_style().font_family = Some(family_name.into());
        self
    }

    fn font_features(mut self, features: FontFeatures) -> Self {
        self.text_style().font_features = Some(features);
        self
    }

    fn font(mut self, font: Font) -> Self {
        let Font {
            family,
            features,
            fallbacks,
            weight,
            style,
        } = font;

        let text_style = self.text_style();
        text_style.font_family = Some(family);
        text_style.font_features = Some(features);
        text_style.font_weight = Some(weight);
        text_style.font_style = Some(style);
        text_style.font_fallbacks = fallbacks;

        self
    }

    fn line_height(mut self, line_height: impl Into<DefiniteLength>) -> Self {
        self.text_style().line_height = Some(line_height.into());
        self
    }

    fn glass(mut self, material: crate::GlassMaterial) -> Self {
        self.style().glass = Some(material);
        self
    }

    fn edge_mark(mut self, mark: crate::EdgeMark) -> Self {
        self.style().edge_mark = Some(mark);
        self
    }

    fn opacity(mut self, opacity: f32) -> Self {
        self.style().opacity = Some(opacity);
        self
    }

    fn grid_cols(mut self, cols: u16) -> Self {
        self.style().grid_cols = Some(GridTemplate {
            repeat: cols,
            min_size: GridTemplateMinSize::Zero,
        });
        self
    }

    fn grid_cols_min_content(mut self, cols: u16) -> Self {
        self.style().grid_cols = Some(GridTemplate {
            repeat: cols,
            min_size: GridTemplateMinSize::MinContent,
        });
        self
    }

    fn grid_cols_max_content(mut self, cols: u16) -> Self {
        self.style().grid_cols = Some(GridTemplate {
            repeat: cols,
            min_size: GridTemplateMinSize::MaxContent,
        });
        self
    }

    fn grid_rows(mut self, rows: u16) -> Self {
        self.style().grid_rows = Some(GridTemplate {
            repeat: rows,
            min_size: GridTemplateMinSize::Zero,
        });
        self
    }

    fn grid_rows_min_content(mut self, rows: u16) -> Self {
        self.style().grid_rows = Some(GridTemplate {
            repeat: rows,
            min_size: GridTemplateMinSize::MinContent,
        });
        self
    }

    fn grid_rows_max_content(mut self, rows: u16) -> Self {
        self.style().grid_rows = Some(GridTemplate {
            repeat: rows,
            min_size: GridTemplateMinSize::MaxContent,
        });
        self
    }

    fn col_start(mut self, start: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.start = GridPlacement::Line(start);
        self
    }

    fn col_start_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.start = GridPlacement::Auto;
        self
    }

    fn col_end(mut self, end: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.end = GridPlacement::Line(end);
        self
    }

    fn col_end_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.end = GridPlacement::Auto;
        self
    }

    fn col_span(mut self, span: u16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column = GridPlacement::Span(span)..GridPlacement::Span(span);
        self
    }

    fn col_span_full(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column = GridPlacement::Line(1)..GridPlacement::Line(-1);
        self
    }

    fn row_start(mut self, start: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.start = GridPlacement::Line(start);
        self
    }

    fn row_start_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.start = GridPlacement::Auto;
        self
    }

    fn row_end(mut self, end: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.end = GridPlacement::Line(end);
        self
    }

    fn row_end_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.end = GridPlacement::Auto;
        self
    }

    fn row_span(mut self, span: u16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row = GridPlacement::Span(span)..GridPlacement::Span(span);
        self
    }

    fn row_span_full(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row = GridPlacement::Line(1)..GridPlacement::Line(-1);
        self
    }

    #[cfg(debug_assertions)]
    fn debug(mut self) -> Self {
        self.style().debug = Some(true);
        self
    }

    #[cfg(debug_assertions)]
    fn debug_below(mut self) -> Self {
        self.style().debug_below = Some(true);
        self
    }
}
