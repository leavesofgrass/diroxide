//! A row of buttons with names and keys: the key bar under the panels,
//! and each panel's sort buttons above its rows.
//!
//! Each button is an AccessKit button node (not a widget) named by its
//! label ("Rename"), with its key as the node's keyboard shortcut (UI
//! Automation's AcceleratorKey, through the vendored adapter's patch), so
//! a screen reader says the name and, when asked, the key; the key is not
//! in the name (textweaver's ADR-0033: the owner found it wordy). The row
//! is a toolbar named for what it holds. The buttons do not take the
//! keyboard focus: Tab is the app's, and every button's key is the
//! keyboard's way to it. A click, or a screen reader's Click on a button,
//! presses it.

use masonry::accesskit::{Action, Node, NodeId, Rect as NodeRect, Role};
use masonry::core::{
    AccessCtx, AccessEvent, BrushIndex, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;

use super::paint;
pub use crate::model::ButtonView;

/// A button was pressed: the row's name and the button's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonPressed {
    /// The row's name ("Key bar").
    pub row: String,
    /// The button's id.
    pub id: String,
}

const HEIGHT: f64 = 30.0;
const PAD: f64 = 8.0;
const GAP: f64 = 6.0;

/// A row of buttons.
pub struct ButtonRow {
    name: String,
    buttons: Vec<ButtonView>,
    layouts: Vec<Layout<BrushIndex>>,
    rects: Vec<Rect>,
    ids: Vec<NodeId>,
    stale: bool,
}

impl ButtonRow {
    /// A toolbar named `name` holding `buttons`.
    pub fn new(name: impl Into<String>, buttons: Vec<ButtonView>) -> Self {
        Self {
            name: name.into(),
            buttons,
            layouts: Vec::new(),
            rects: Vec::new(),
            ids: Vec::new(),
            stale: true,
        }
    }

    /// The buttons shown.
    pub fn buttons(&self) -> &[ButtonView] {
        &self.buttons
    }

    /// Shows other buttons (another bank, another key).
    pub fn set_buttons(this: &mut WidgetMut<'_, Self>, buttons: Vec<ButtonView>) {
        if this.widget.buttons != buttons {
            this.widget.buttons = buttons;
            this.widget.stale = true;
            this.ctx.request_layout();
            this.ctx.request_render();
        }
    }

    fn press(&self, ctx: &mut EventCtx<'_>, index: usize) {
        if let Some(button) = self.buttons.get(index) {
            ctx.submit_action::<ButtonPressed>(ButtonPressed {
                row: self.name.clone(),
                id: button.id.clone(),
            });
            ctx.set_handled();
        }
    }
}

impl Widget for ButtonRow {
    type Action = ButtonPressed;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn property_changed(
        &mut self,
        ctx: &mut masonry::core::UpdateCtx<'_>,
        property_type: std::any::TypeId,
    ) {
        if paint::is_colors(property_type) {
            ctx.request_render();
        }
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if let PointerEvent::Down(button) = event {
            let at = ctx.local_position(button.state.position);
            if let Some(index) = self.rects.iter().position(|r| r.contains(at)) {
                self.press(ctx, index);
            }
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        if event.action != Action::Click {
            return;
        }
        if let Some(index) = event
            .node
            .and_then(|node| self.ids.iter().position(|id| *id == node))
        {
            self.press(ctx, index);
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
            (Axis::Horizontal, LenReq::FitContent(space)) => space,
            (Axis::Horizontal, _) => Length::px(640.0),
            (Axis::Vertical, _) => {
                if self.buttons.is_empty() {
                    Length::ZERO
                } else {
                    Length::px(HEIGHT)
                }
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        if self.stale || self.layouts.len() != self.buttons.len() {
            self.layouts = self
                .buttons
                .iter()
                .map(|b| paint::line_layout(ctx, &b.text(), paint::TEXT_SIZE - 1.0, false))
                .collect();
            self.stale = false;
        }
        // Buttons that do not fit the row's width are not drawn (Say Key
        // Bar says every pair); their nodes stay, named and pressable.
        self.rects.clear();
        let mut x = 2.0;
        for layout in &self.layouts {
            let w = f64::from(layout.width()) + 2.0 * PAD;
            self.rects.push(Rect::new(x, 2.0, x + w, HEIGHT - 2.0));
            x += w + GAP;
        }
        let _ = size;
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let palette = paint::palette_of(props, ctx.property_cache());
        let look = palette.look(diroxide_theme::PanelRole::KeyBar);
        let width = ctx.content_box().size().width;
        for (rect, layout) in self.rects.iter().zip(&self.layouts) {
            if rect.x1 > width {
                break;
            }
            painter.fill(*rect, paint::color(look.bg)).draw();
            painter
                .stroke(*rect, &Stroke::new(1.0), paint::color(look.fg))
                .draw();
            let y = rect.y0 + (rect.height() - f64::from(layout.height())) / 2.0;
            render_text(
                painter,
                Affine::translate(Vec2::new(rect.x0 + PAD, y)),
                layout,
                &[paint::color(look.fg).into()],
                true,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Toolbar
    }

    fn accessibility(
        &mut self,
        ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.name.as_str());
        node.clear_clips_children();
        while self.ids.len() < self.buttons.len() {
            self.ids.push(AccessCtx::next_node_id());
        }
        let mut children = Vec::with_capacity(self.buttons.len());
        for (i, button) in self.buttons.iter().enumerate() {
            let mut node = Node::new(Role::Button);
            node.set_label(button.label.as_str());
            if let Some(key) = &button.key {
                node.set_keyboard_shortcut(key.as_str());
            }
            node.add_action(Action::Click);
            if let Some(rect) = self.rects.get(i) {
                node.set_bounds(NodeRect::new(rect.x0, rect.y0, rect.x1, rect.y1));
            }
            ctx.tree_update().nodes.push((self.ids[i], node));
            children.push(self.ids[i]);
        }
        node.set_children(children);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
