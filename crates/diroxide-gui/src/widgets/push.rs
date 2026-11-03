//! A button or a toggle inside a dialog: a confirmation's No and Yes, a
//! secret's Show. Unlike the key bar's buttons, these take the keyboard
//! focus, since a dialog's answer is a control a screen reader lands on:
//! the focus starts on No, and Tab moves between them.
//!
//! - The node is a button named by its label ("No"), or a check box for
//!   a toggle, with its state; the key is the node's keyboard shortcut
//!   (UI Automation's AcceleratorKey), never in the name (textweaver's
//!   ADR-0033).
//! - Keys are the app's, through the window's root: the button handles
//!   none, so Enter, Space, y and n reach the driver, which answers for
//!   the focused button.
//! - A click, or a screen reader's Click, presses it; a screen reader's
//!   Focus moves the window's focus to it. Either tells the driver, which
//!   keeps the dialog's focus in step.
//!
//! Drawn as a framed label, a thick ring when focused and a box with a
//! mark for a toggle that is on, so neither the focus nor the state is
//! shown by color alone.

use masonry::accesskit::{Action, Node, Role, Toggled};
use masonry::core::{
    AccessCtx, AccessEvent, BrushIndex, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetMut,
    render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Line, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;

use super::paint;
use crate::model::{DialogFocus, PushView};

/// The button's height.
pub const HEIGHT: f64 = 34.0;
const PAD: f64 = 14.0;
/// A toggle's box.
const BOX: f64 = 16.0;

/// What a dialog's button tells the driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushAction {
    /// Pressed: a click, or a screen reader's Click.
    Pressed(DialogFocus),
    /// Took the keyboard focus another way than the driver's (a click, a
    /// screen reader's Focus): the dialog's focus follows.
    Focused(DialogFocus),
}

/// A button or toggle in a dialog.
pub struct PushButton {
    view: PushView,
    focused: bool,
    layout: Option<Layout<BrushIndex>>,
}

impl PushButton {
    /// A button showing `view`.
    pub fn new(view: PushView) -> Self {
        Self {
            view,
            focused: false,
            layout: None,
        }
    }

    /// What the button shows.
    pub fn view(&self) -> &PushView {
        &self.view
    }

    /// Shows another label, key or state.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: PushView) {
        if this.widget.view != view {
            if this.widget.view.label != view.label {
                this.widget.layout = None;
                this.ctx.request_layout();
            }
            this.widget.view = view;
            this.ctx.request_render();
            this.ctx.request_accessibility_update();
        }
    }

    /// The text drawn: the label, and the key after it ("No (n)").
    fn drawn(&self) -> String {
        match &self.view.key {
            Some(key) => format!("{} ({key})", self.view.label),
            None => self.view.label.clone(),
        }
    }
}

impl Widget for PushButton {
    type Action = PushAction;

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn property_changed(&mut self, ctx: &mut UpdateCtx<'_>, property_type: std::any::TypeId) {
        if paint::is_colors(property_type) {
            ctx.request_render();
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::FocusChanged(focused) = event {
            self.focused = *focused;
            if *focused {
                ctx.submit_action::<PushAction>(PushAction::Focused(self.view.id));
            }
            ctx.request_render();
        }
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        if let PointerEvent::Down(_) = event {
            ctx.request_focus();
            ctx.submit_action::<PushAction>(PushAction::Pressed(self.view.id));
            ctx.set_handled();
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        match event.action {
            Action::Click => {
                ctx.request_focus();
                ctx.submit_action::<PushAction>(PushAction::Pressed(self.view.id));
                ctx.set_handled();
            }
            Action::Focus => {
                ctx.request_focus();
                ctx.set_handled();
            }
            _ => {}
        }
    }

    fn measure(
        &mut self,
        ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        _len_req: LenReq,
        _cross: Option<Length>,
    ) -> Length {
        match axis {
            Axis::Horizontal => {
                if self.layout.is_none() {
                    let text = self.drawn();
                    self.layout = Some(paint::measured_layout(ctx, &text, paint::TEXT_SIZE, false));
                }
                let text = self
                    .layout
                    .as_ref()
                    .map_or(0.0, |layout| f64::from(layout.width()));
                let toggle = if self.view.toggled.is_some() {
                    BOX + 8.0
                } else {
                    0.0
                };
                Length::px((text + toggle + 2.0 * PAD).ceil())
            }
            Axis::Vertical => Length::px(HEIGHT),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, _size: Size) {
        if self.layout.is_none() {
            let text = self.drawn();
            self.layout = Some(paint::line_layout(ctx, &text, paint::TEXT_SIZE, false));
        }
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let palette = paint::palette_of(props, ctx.property_cache());
        let band = palette.look(diroxide_theme::PanelRole::FocusedRow);
        let card = palette.look(diroxide_theme::PanelRole::Dialog);
        let size = ctx.content_box().size();
        let frame = Rect::new(1.0, 1.0, size.width - 1.0, size.height - 1.0);
        if self.focused {
            painter.fill(frame, paint::color(band.bg)).draw();
        }
        let ring = if self.focused { 3.0 } else { 1.0 };
        painter
            .stroke(frame, &Stroke::new(ring), paint::color(palette.ring))
            .draw();
        if self.focused {
            // The line inside the ring, so it shows on the band too.
            painter
                .stroke(
                    frame.inset(-2.0),
                    &Stroke::new(1.0),
                    paint::color(palette.ring_inner),
                )
                .draw();
        }
        let color = paint::color(if self.focused { band.fg } else { card.fg });
        let mut x = PAD;
        if let Some(on) = self.view.toggled {
            // The toggle's box, with a cross when it is on: a shape, so
            // the state is never color alone.
            let y = (size.height - BOX) / 2.0;
            let mark = Rect::new(x, y, x + BOX, y + BOX);
            painter.stroke(mark, &Stroke::new(1.5), color).draw();
            if on {
                let inner = mark.inset(-3.0);
                painter
                    .stroke(
                        Line::new((inner.x0, inner.y0), (inner.x1, inner.y1)),
                        &Stroke::new(2.0),
                        color,
                    )
                    .draw();
                painter
                    .stroke(
                        Line::new((inner.x0, inner.y1), (inner.x1, inner.y0)),
                        &Stroke::new(2.0),
                        color,
                    )
                    .draw();
            }
            x += BOX + 8.0;
        }
        if let Some(layout) = &self.layout {
            let y = (size.height - f64::from(layout.height())) / 2.0;
            render_text(
                painter,
                Affine::translate(Vec2::new(x, y)),
                layout,
                &[color.into()],
                true,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        if self.view.toggled.is_some() {
            Role::CheckBox
        } else {
            Role::Button
        }
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.view.label.as_str());
        if let Some(key) = &self.view.key {
            node.set_keyboard_shortcut(key.as_str());
        }
        if let Some(on) = self.view.toggled {
            node.set_toggled(if on { Toggled::True } else { Toggled::False });
        }
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
