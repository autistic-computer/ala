use crate::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Point, Window,
};

/// An element painted scaled around its own origin and then moved, with its
/// layout and hit testing left as they are. See [`PaintScaleExt::paint_scale`].
pub struct PaintScale<E> {
    element: Option<E>,
    scale: f32,
    offset: Point<Pixels>,
}

/// Paint an element scaled without laying it out again.
pub trait PaintScaleExt {
    /// Paint this element scaled by `scale` around its top-left corner, then
    /// moved by `offset`, clipped to the content mask it is painted in. Mouse
    /// input still lands where the element is laid out.
    fn paint_scale(self, scale: f32, offset: Point<Pixels>) -> PaintScale<Self>
    where
        Self: Sized,
    {
        PaintScale {
            element: Some(self),
            scale,
            offset,
        }
    }
}

impl<E: IntoElement + 'static> PaintScaleExt for E {}

impl<E: IntoElement + 'static> IntoElement for PaintScale<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: IntoElement + 'static> Element for PaintScale<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut element = self
            .element
            .take()
            .expect("should only be called once")
            .into_any_element();
        (element.request_layout(window, cx), element)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_paint_scale(bounds.origin, self.scale, self.offset, |window| {
            element.paint(window, cx)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::PaintScaleExt as _;
    use crate::{
        AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
        ScaledPixels, StyleRefinement, Styled as _, TestAppContext, Window, div, hsla, point, px,
        size,
    };

    struct Tile;

    impl Render for Tile {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .absolute()
                    .left(px(20.))
                    .top(px(40.))
                    .w(px(100.))
                    .h(px(50.))
                    .rounded(px(10.))
                    .bg(hsla(0.6, 0.5, 0.5, 1.)),
            )
        }
    }

    struct Scaled {
        tile: Entity<Tile>,
        scale: f32,
    }

    impl Render for Scaled {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .size_full()
                    .child(
                        self.tile
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    )
                    .paint_scale(self.scale, point(px(10.), px(10.))),
            )
        }
    }

    fn painted(window: &Window) -> (Bounds<ScaledPixels>, ScaledPixels) {
        let quads = window.painted_quads();
        assert_eq!(quads.len(), 1);
        (quads[0].bounds, quads[0].corner_radii.top_left)
    }

    #[crate::test]
    fn scales_what_it_paints_and_keeps_scaling_cached_views(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| Scaled {
            tile: cx.new(|_| Tile),
            scale: 0.5,
        });
        cx.run_until_parked();
        let factor = cx.update(|window, _| window.scale_factor());
        let expected = |x: f32, y: f32, w: f32, h: f32| Bounds {
            origin: point(ScaledPixels(x * factor), ScaledPixels(y * factor)),
            size: size(ScaledPixels(w * factor), ScaledPixels(h * factor)),
        };

        let (bounds, radius) = cx.update(|window, _| painted(window));
        assert_eq!(bounds, expected(20., 30., 50., 25.));
        assert_eq!(radius, ScaledPixels(5. * factor));

        view.update(cx, |view, cx| {
            view.scale = 2.;
            cx.notify();
        });
        cx.run_until_parked();
        let (bounds, _) = cx.update(|window, _| painted(window));
        assert_eq!(bounds, expected(50., 90., 200., 100.));

        view.update(cx, |view, cx| {
            view.scale = 1.;
            cx.notify();
        });
        cx.run_until_parked();
        let (bounds, _) = cx.update(|window, _| painted(window));
        assert_eq!(bounds, expected(30., 50., 100., 50.));
    }
}
