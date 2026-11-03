//! The window's body: the two panels, or the viewer in their place while
//! a file is viewed, as the terminal's viewer takes its body. The one not
//! shown is stashed, so it is out of the tree and out of the focus order.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Point, Size};
use masonry::layout::{LayoutSize, LenReq, Length};

/// The panels or the viewer.
pub struct Screen {
    panels: WidgetPod<dyn Widget>,
    viewer: WidgetPod<dyn Widget>,
    viewing: bool,
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
            viewing: false,
        }
    }

    /// Shows the viewer (`true`) or the panels.
    pub fn set_viewing(this: &mut WidgetMut<'_, Self>, viewing: bool) {
        if this.widget.viewing != viewing {
            this.widget.viewing = viewing;
            this.ctx.set_stashed(&mut this.widget.panels, viewing);
            this.ctx.set_stashed(&mut this.widget.viewer, !viewing);
            this.ctx.request_layout();
        }
    }

    /// Whether the viewer is shown.
    pub fn viewing(&self) -> bool {
        self.viewing
    }
}

impl Widget for Screen {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.panels);
        ctx.register_child(&mut self.viewer);
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
        let shown = if self.viewing {
            &mut self.viewer
        } else {
            &mut self.panels
        };
        ctx.compute_length(shown, len_req.into(), context, axis, cross)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        ctx.set_stashed(&mut self.panels, self.viewing);
        ctx.set_stashed(&mut self.viewer, !self.viewing);
        let shown = if self.viewing {
            &mut self.viewer
        } else {
            &mut self.panels
        };
        ctx.run_layout(shown, size);
        ctx.place_child(shown, Point::ORIGIN);
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _p: &mut Painter<'_>) {
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
        ChildrenIds::from_slice(&[self.panels.id(), self.viewer.id()])
    }
}
