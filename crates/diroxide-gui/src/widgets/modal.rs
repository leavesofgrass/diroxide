//! The modal host: the top dialog of the app's stack over the window, as
//! one dialog node named by the dialog's title (the question, the list's
//! title, the field's label), with the lines to read first and then the
//! one control the focus is on: a text field with its candidates, or a
//! list. Only the top dialog is shown; when it closes the one beneath
//! shows, and when the last closes the focus returns to the panel's
//! focused row (the list box's active descendant).
//!
//! The dialog shapes' own controls (a confirmation's buttons, the secret
//! field's Show toggle, the Jobs list's actions) are the dialogs lane's
//! (R27's commits 17 to 25); here every shape is a name, its lines and
//! its control, and every key is the app's.

use masonry::accesskit::{Node, NodeId, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod, WidgetTag,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Point, Rect, Size, Stroke};
use masonry::layout::{LayoutSize, LenDef, LenReq, Length};
use masonry::peniko::Color;
use masonry::widgets::{Flex, Label};

use super::choice::ChoiceBox;
use super::field::Field;
use super::paint;
use crate::model::{ControlView, DialogView};

/// The dialog's text field.
pub const DIALOG_FIELD: WidgetTag<Field> = WidgetTag::named("diroxide-dialog-field");
/// The dialog's list (a choice, a text to read, a field's candidates).
pub const DIALOG_LIST: WidgetTag<ChoiceBox> = WidgetTag::named("diroxide-dialog-list");

const CARD_WIDTH: f64 = 640.0;
const MARGIN: f64 = 24.0;
const PAD: f64 = 16.0;
const GAP: f64 = 10.0;

/// The page behind a dialog, darkened, so the card stands out; the window
/// behind is also hidden from screen readers.
const VEIL: Color = Color::from_rgba8(0, 0, 0, 0xb0);

/// A dialog over the window.
pub struct Modal {
    view: DialogView,
    lines: WidgetPod<Flex>,
    field: Option<WidgetPod<Field>>,
    list: Option<WidgetPod<ChoiceBox>>,
    /// Whether the list is shown (a field's candidates may be closed).
    list_shown: bool,
    card: Rect,
}

impl Modal {
    /// The modal for `view`.
    pub fn new(view: DialogView) -> Self {
        let mut lines = Flex::column();
        for line in &view.lines {
            lines = lines.with_fixed(NewWidget::new(Label::new(line.clone())));
        }
        let (field, list, list_shown) = match &view.control {
            ControlView::Field(field) => {
                let candidates =
                    field
                        .candidates
                        .clone()
                        .unwrap_or_else(|| crate::model::ListView {
                            name: "Candidates".to_owned(),
                            first: 0,
                            rows: Vec::new(),
                            total: 0,
                            focus: None,
                        });
                let list = NewWidget::new(ChoiceBox::new(candidates)).with_tag(DIALOG_LIST);
                let list_id: NodeId = list.id().into();
                let field =
                    NewWidget::new(Field::new(field.clone(), Some(list_id))).with_tag(DIALOG_FIELD);
                (
                    Some(field.to_pod()),
                    Some(list.to_pod()),
                    field_has_list(&view),
                )
            }
            ControlView::List(list) => {
                let list = NewWidget::new(ChoiceBox::new(list.clone())).with_tag(DIALOG_LIST);
                (None, Some(list.to_pod()), true)
            }
        };
        Self {
            view,
            lines: NewWidget::new(lines).to_pod(),
            field,
            list,
            list_shown,
            card: Rect::ZERO,
        }
    }

    /// What the modal shows.
    pub fn view(&self) -> &DialogView {
        &self.view
    }

    /// The widget the keyboard focus belongs on: the field, or the list
    /// (a list shape, or a field's candidates while the focus is in
    /// them).
    pub fn focus_id(&self) -> Option<masonry::core::WidgetId> {
        let in_list = match &self.view.control {
            ControlView::Field(field) => field.in_list && field.candidates.is_some(),
            ControlView::List(_) => true,
        };
        if in_list {
            self.list.as_ref().map(WidgetPod::id)
        } else {
            self.field.as_ref().map(WidgetPod::id)
        }
    }

    /// Shows the same dialog as it is now (a typed character, a moved
    /// focus, candidates listed): the control is changed in place, so
    /// its node and the focus stay.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: DialogView) {
        let shown = field_has_list(&view);
        match &view.control {
            ControlView::Field(field) => {
                if let Some(pod) = this.widget.field.as_mut() {
                    let mut widget = this.ctx.get_mut(pod);
                    Field::set_view(&mut widget, field.clone());
                }
                if let (Some(candidates), Some(pod)) =
                    (field.candidates.clone(), this.widget.list.as_mut())
                {
                    let mut widget = this.ctx.get_mut(pod);
                    ChoiceBox::set_view(&mut widget, candidates);
                }
            }
            ControlView::List(list) => {
                if let Some(pod) = this.widget.list.as_mut() {
                    let mut widget = this.ctx.get_mut(pod);
                    ChoiceBox::set_view(&mut widget, list.clone());
                }
            }
        }
        if shown != this.widget.list_shown {
            this.widget.list_shown = shown;
            this.ctx.request_layout();
        }
        this.widget.view = view;
        this.ctx.request_render();
    }
}

/// Whether the dialog's list is shown: always for a list, while the
/// candidates are open for a field.
fn field_has_list(view: &DialogView) -> bool {
    match &view.control {
        ControlView::Field(field) => field.candidates.is_some(),
        ControlView::List(_) => true,
    }
}

impl Widget for Modal {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.lines);
        if let Some(field) = &mut self.field {
            ctx.register_child(field);
        }
        if let Some(list) = &mut self.list {
            ctx.register_child(list);
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
            (Axis::Horizontal, _) => Length::px(CARD_WIDTH),
            (Axis::Vertical, _) => Length::px(400.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let width = (size.width - 2.0 * MARGIN).clamp(120.0, CARD_WIDTH);
        let inner = width - 2.0 * PAD;
        let context = LayoutSize::maybe(Axis::Horizontal, Some(Length::px(inner)));
        let lines_h = ctx
            .compute_length(
                &mut self.lines,
                LenDef::FitContent(Length::px(size.height)),
                context,
                Axis::Vertical,
                Some(Length::px(inner)),
            )
            .get();
        let mut y = PAD;
        let x0 = ((size.width - width) / 2.0).max(0.0);
        let top = MARGIN;
        ctx.run_layout(&mut self.lines, Size::new(inner, lines_h));
        ctx.place_child(&mut self.lines, Point::new(x0 + PAD, top + y));
        y += lines_h + if lines_h > 0.0 { GAP } else { 0.0 };
        if let Some(field) = &mut self.field {
            let h = super::field::FIELD_HEIGHT;
            ctx.run_layout(field, Size::new(inner, h));
            ctx.place_child(field, Point::new(x0 + PAD, top + y));
            y += h + GAP;
        }
        if let Some(list) = &mut self.list {
            ctx.set_stashed(list, !self.list_shown);
            if self.list_shown {
                let room = (size.height - top - y - PAD - MARGIN).max(28.0);
                let h = ctx
                    .compute_length(
                        list,
                        LenDef::FitContent(Length::px(room)),
                        context,
                        Axis::Vertical,
                        Some(Length::px(inner)),
                    )
                    .get()
                    .min(room);
                ctx.run_layout(list, Size::new(inner, h));
                ctx.place_child(list, Point::new(x0 + PAD, top + y));
                y += h;
            }
        }
        self.card = Rect::new(x0, top, x0 + width, top + y + PAD);
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let size = ctx.content_box().size();
        painter.fill(size.to_rect(), VEIL).draw();
        painter.fill(self.card, paint::PAGE).draw();
        painter
            .stroke(self.card, &Stroke::new(2.0), paint::RING)
            .draw();
    }

    fn accessibility_role(&self) -> Role {
        Role::Dialog
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.view.title.as_str());
        node.set_modal();
    }

    fn children_ids(&self) -> ChildrenIds {
        let mut ids = ChildrenIds::from_slice(&[self.lines.id()]);
        if let Some(field) = &self.field {
            ids.push(field.id());
        }
        if let Some(list) = &self.list {
            ids.push(list.id());
        }
        ids
    }
}
