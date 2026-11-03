//! The two panels in the `layout` setting's arrangement: side by side
//! (the default, as in every orthodox manager), stacked (one above the
//! other, each row at full width: the fallback for a screen reader's
//! line and a 40-cell display), or single (the active panel alone; the
//! other is stashed, out of the drawing and the tree, until Tab).

use diroxide_app::config::Layout;
use diroxide_core::Side;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Point, Size};
use masonry::layout::{LenReq, Length};

use super::panel::PanelBox;

/// The gap between the panels, in logical pixels.
const GAP: f64 = 6.0;

/// The two panels.
pub struct Panels {
    left: WidgetPod<PanelBox>,
    right: WidgetPod<PanelBox>,
    layout: Layout,
    active: Side,
}

impl Panels {
    /// The panels in `layout`, `active` the one shown alone in single.
    pub fn new(
        left: NewWidget<PanelBox>,
        right: NewWidget<PanelBox>,
        layout: Layout,
        active: Side,
    ) -> Self {
        Self {
            left: left.to_pod(),
            right: right.to_pod(),
            layout,
            active,
        }
    }

    /// The arrangement.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Arranges the panels in `layout`, with `active` shown in single.
    pub fn arrange(this: &mut WidgetMut<'_, Self>, layout: Layout, active: Side) {
        if this.widget.layout == layout && this.widget.active == active {
            return;
        }
        this.widget.layout = layout;
        this.widget.active = active;
        this.ctx.request_layout();
    }

    /// One panel's list box.
    pub fn panel_mut<'t>(this: &'t mut WidgetMut<'_, Self>, side: Side) -> WidgetMut<'t, PanelBox> {
        match side {
            Side::Left => this.ctx.get_mut(&mut this.widget.left),
            Side::Right => this.ctx.get_mut(&mut this.widget.right),
        }
    }
}

impl Widget for Panels {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.left);
        ctx.register_child(&mut self.right);
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        _cross: Option<Length>,
    ) -> Length {
        match (axis, len_req) {
            (_, LenReq::FitContent(space)) => space,
            (Axis::Horizontal, _) => Length::px(640.0),
            (Axis::Vertical, _) => Length::px(400.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        // In single the other panel is stashed: not drawn, not in the
        // tree, and not laid out.
        let single = self.layout == Layout::Single;
        ctx.set_stashed(&mut self.left, single && self.active != Side::Left);
        ctx.set_stashed(&mut self.right, single && self.active != Side::Right);
        match self.layout {
            Layout::SideBySide => {
                let w = ((size.width - GAP) / 2.0).max(0.0);
                ctx.run_layout(&mut self.left, Size::new(w, size.height));
                ctx.place_child(&mut self.left, Point::ORIGIN);
                ctx.run_layout(&mut self.right, Size::new(w, size.height));
                ctx.place_child(&mut self.right, Point::new(w + GAP, 0.0));
            }
            Layout::Stacked => {
                let h = ((size.height - GAP) / 2.0).max(0.0);
                ctx.run_layout(&mut self.left, Size::new(size.width, h));
                ctx.place_child(&mut self.left, Point::ORIGIN);
                ctx.run_layout(&mut self.right, Size::new(size.width, h));
                ctx.place_child(&mut self.right, Point::new(0.0, h + GAP));
            }
            Layout::Single => {
                let shown = match self.active {
                    Side::Left => &mut self.left,
                    Side::Right => &mut self.right,
                };
                ctx.run_layout(shown, size);
                ctx.place_child(shown, Point::ORIGIN);
            }
        }
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
        ChildrenIds::from_slice(&[self.left.id(), self.right.id()])
    }
}
