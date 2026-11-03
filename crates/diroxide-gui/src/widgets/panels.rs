//! The two panels in the `layout` setting's arrangement: side by side
//! (the default, as in every orthodox manager), stacked (one above the
//! other, each row at full width: the fallback for a screen reader's
//! line and a 40-cell display), or single (the active panel alone; the
//! other is stashed, out of the drawing and the tree, until Tab). Each
//! panel is its sort buttons above its list box.

use diroxide_app::config::Layout;
use diroxide_core::Side;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Point, Rect, Size};
use masonry::layout::{LenReq, Length};

use super::buttons::ButtonRow;
use super::panel::PanelBox;

/// The gap between the panels, in logical pixels.
const GAP: f64 = 6.0;
/// The sort buttons' height.
const SORT_HEIGHT: f64 = 32.0;

/// One panel: its sort buttons and its list box.
struct PanelPods {
    sort: WidgetPod<ButtonRow>,
    list: WidgetPod<PanelBox>,
}

/// The two panels.
pub struct Panels {
    left: PanelPods,
    right: PanelPods,
    layout: Layout,
    active: Side,
}

impl Panels {
    /// The panels, each its sort buttons and its list box, in `layout`,
    /// `active` the one shown alone in single.
    pub fn new(
        left: (NewWidget<ButtonRow>, NewWidget<PanelBox>),
        right: (NewWidget<ButtonRow>, NewWidget<PanelBox>),
        layout: Layout,
        active: Side,
    ) -> Self {
        let side = |(sort, list): (NewWidget<ButtonRow>, NewWidget<PanelBox>)| PanelPods {
            sort: sort.to_pod(),
            list: list.to_pod(),
        };
        Self {
            left: side(left),
            right: side(right),
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
}

/// Lays out one panel in `area`: the sort buttons on top, the list box
/// under them.
fn place(ctx: &mut LayoutCtx<'_>, side: &mut PanelPods, area: Rect) {
    let sort_h = SORT_HEIGHT.min(area.height());
    ctx.run_layout(&mut side.sort, Size::new(area.width(), sort_h));
    ctx.place_child(&mut side.sort, Point::new(area.x0, area.y0));
    let list_h = (area.height() - sort_h).max(0.0);
    ctx.run_layout(&mut side.list, Size::new(area.width(), list_h));
    ctx.place_child(&mut side.list, Point::new(area.x0, area.y0 + sort_h));
}

fn stash(ctx: &mut LayoutCtx<'_>, side: &mut PanelPods, stashed: bool) {
    ctx.set_stashed(&mut side.sort, stashed);
    ctx.set_stashed(&mut side.list, stashed);
}

impl Widget for Panels {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for side in [&mut self.left, &mut self.right] {
            ctx.register_child(&mut side.sort);
            ctx.register_child(&mut side.list);
        }
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
        stash(ctx, &mut self.left, single && self.active != Side::Left);
        stash(ctx, &mut self.right, single && self.active != Side::Right);
        match self.layout {
            Layout::SideBySide => {
                let w = ((size.width - GAP) / 2.0).max(0.0);
                place(ctx, &mut self.left, Rect::new(0.0, 0.0, w, size.height));
                place(
                    ctx,
                    &mut self.right,
                    Rect::new(w + GAP, 0.0, w + GAP + w, size.height),
                );
            }
            Layout::Stacked => {
                let h = ((size.height - GAP) / 2.0).max(0.0);
                place(ctx, &mut self.left, Rect::new(0.0, 0.0, size.width, h));
                place(
                    ctx,
                    &mut self.right,
                    Rect::new(0.0, h + GAP, size.width, h + GAP + h),
                );
            }
            Layout::Single => {
                let shown = match self.active {
                    Side::Left => &mut self.left,
                    Side::Right => &mut self.right,
                };
                place(ctx, shown, size.to_rect());
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
        ChildrenIds::from_slice(&[
            self.left.sort.id(),
            self.left.list.id(),
            self.right.sort.id(),
            self.right.list.id(),
        ])
    }
}
