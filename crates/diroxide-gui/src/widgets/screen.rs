//! The window's body: the two panels, the viewer in their place while a
//! file is viewed, as the terminal's viewer takes its body, or the
//! picture view while a picture is open (alpha 4, R30 commit 112). The
//! ones not shown are stashed, so they are out of the tree and out of the
//! focus order. Its place in the window is kept for focus mode's veil
//! ([`super::root::Root::set_veiled`]), which covers everything else.

use std::cell::Cell;
use std::rc::Rc;

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Point, Rect, Size};
use masonry::layout::{LayoutSize, LenReq, Length};

/// What the body shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Body {
    /// The two panels.
    #[default]
    Panels,
    /// The viewer.
    Viewer,
    /// The picture view.
    Picture,
}

/// The panels, the viewer, or the picture view.
pub struct Screen {
    panels: WidgetPod<dyn Widget>,
    viewer: WidgetPod<dyn Widget>,
    picture: Option<WidgetPod<dyn Widget>>,
    body: Body,
    /// Where the body is in the window, as last painted.
    area: Rc<Cell<Rect>>,
}

impl Screen {
    /// The body over `panels` and `viewer`, the panels shown.
    pub fn new(
        panels: NewWidget<impl Widget + ?Sized>,
        viewer: NewWidget<impl Widget + ?Sized>,
    ) -> Self {
        Self {
            panels: panels.erased().to_pod(),
            viewer: viewer.erased().to_pod(),
            picture: None,
            body: Body::Panels,
            area: Rc::new(Cell::new(Rect::ZERO)),
        }
    }

    /// The same body with the picture view as its third.
    #[must_use]
    pub fn with_picture(mut self, picture: NewWidget<impl Widget + ?Sized>) -> Self {
        self.picture = Some(picture.erased().to_pod());
        self
    }

    /// The same body telling `area` where it is in the window.
    #[must_use]
    pub fn with_area(mut self, area: Rc<Cell<Rect>>) -> Self {
        self.area = area;
        self
    }

    /// Shows the viewer (`true`) or the panels.
    pub fn set_viewing(this: &mut WidgetMut<'_, Self>, viewing: bool) {
        Self::show(this, if viewing { Body::Viewer } else { Body::Panels });
    }

    /// Shows `body`; the picture view only when the body has one.
    pub fn show(this: &mut WidgetMut<'_, Self>, body: Body) {
        let body = if body == Body::Picture && this.widget.picture.is_none() {
            Body::Panels
        } else {
            body
        };
        if this.widget.body != body {
            this.widget.body = body;
            this.ctx
                .set_stashed(&mut this.widget.panels, body != Body::Panels);
            this.ctx
                .set_stashed(&mut this.widget.viewer, body != Body::Viewer);
            if let Some(picture) = this.widget.picture.as_mut() {
                this.ctx.set_stashed(picture, body != Body::Picture);
            }
            this.ctx.request_layout();
        }
    }

    /// Whether the viewer is shown.
    pub fn viewing(&self) -> bool {
        self.body == Body::Viewer
    }

    /// What the body shows.
    pub fn body(&self) -> Body {
        self.body
    }

    fn shown(&mut self) -> &mut WidgetPod<dyn Widget> {
        match (self.body, self.picture.as_mut()) {
            (Body::Picture, Some(picture)) => picture,
            (Body::Viewer, _) => &mut self.viewer,
            _ => &mut self.panels,
        }
    }
}

impl Widget for Screen {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.panels);
        ctx.register_child(&mut self.viewer);
        if let Some(picture) = self.picture.as_mut() {
            ctx.register_child(picture);
        }
    }

    fn measure(
        &mut self,
        ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        cross: Option<Length>,
    ) -> Length {
        let context = LayoutSize::maybe(axis.cross(), cross);
        let shown = self.shown();
        ctx.compute_length(shown, len_req.into(), context, axis, cross)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let body = self.body;
        ctx.set_stashed(&mut self.panels, body != Body::Panels);
        ctx.set_stashed(&mut self.viewer, body != Body::Viewer);
        if let Some(picture) = self.picture.as_mut() {
            ctx.set_stashed(picture, body != Body::Picture);
        }
        let shown = self.shown();
        ctx.run_layout(shown, size);
        ctx.place_child(shown, Point::ORIGIN);
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _p: &mut Painter<'_>) {
        let area = ctx
            .window_transform()
            .transform_rect_bbox(ctx.content_box());
        self.area.set(area);
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        let mut ids = vec![self.panels.id(), self.viewer.id()];
        if let Some(picture) = &self.picture {
            ids.push(picture.id());
        }
        ChildrenIds::from_slice(&ids)
    }
}
