//! The modal host: the top dialog of the app's stack over the window, as
//! one dialog node named by the dialog's title (the question, the list's
//! title, the field's label), with the lines to read first and then the
//! controls: a text field with its candidates, a list, a confirmation's
//! listing, and the buttons (a confirmation's No and Yes, a secret's Show
//! toggle). Only the top dialog is shown, and it alone is in the tree;
//! when it closes the one beneath shows, and when the last closes the
//! focus returns to the panel's focused row (the list box's active
//! descendant).
//!
//! - The title is drawn at the top of the card and is not a node of its
//!   own: the dialog's name says it, so a screen reader hears it once.
//! - A question's situation after its first line is the dialog's
//!   description, which a screen reader reads after the name as the
//!   dialog opens, and it is drawn as lines too.
//! - Every key is the app's, through the window's root; the driver keeps
//!   the window's own focus (a confirmation's buttons and listing, a
//!   text's lines, a secret's toggle), which the app does not model.
//!
//! The card's shape follows textweaver's `crates/textweaver-xilem/src/dialog.rs`
//! (`Modal`: a card over a dimmed page, the `Dialog` role, a name and a
//! description; the owner's own project).

use masonry::accesskit::{Node, NodeId, Role};
use masonry::core::{
    AccessCtx, BrushIndex, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, NewWidget, NoAction,
    PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget, WidgetId, WidgetMut,
    WidgetPod, WidgetTag, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, Size, Stroke, Vec2};
use masonry::layout::{LayoutSize, LenDef, LenReq, Length};
use masonry::parley::Layout;
use masonry::widgets::{Flex, Label};

use super::choice::ChoiceBox;
use super::field::Field;
use super::paint;
use super::push::PushButton;
use crate::model::{ControlView, DialogFocus, DialogView, ListView};

/// The dialog's text field.
pub const DIALOG_FIELD: WidgetTag<Field> = WidgetTag::named("diroxide-dialog-field");
/// The dialog's list (a choice, a text to read, a field's candidates, a
/// confirmation's listing).
pub const DIALOG_LIST: WidgetTag<ChoiceBox> = WidgetTag::named("diroxide-dialog-list");

const CARD_WIDTH: f64 = 640.0;
const MARGIN: f64 = 24.0;
const PAD: f64 = 16.0;
const GAP: f64 = 10.0;
/// The title's size: larger than a row's, bold.
const TITLE_SIZE: f32 = 19.0;

/// The page behind a dialog, darkened, so the card stands out; the window
/// behind is also hidden from screen readers.
/// How much of the window behind a dialog shows through the veil.
const VEIL: f32 = 0.7;

/// A dialog over the window.
pub struct Modal {
    view: DialogView,
    title: Option<Layout<BrushIndex>>,
    lines: WidgetPod<Flex>,
    field: Option<WidgetPod<Field>>,
    list: Option<WidgetPod<ChoiceBox>>,
    buttons: Vec<WidgetPod<PushButton>>,
    /// Whether the list is shown (a field's candidates may be closed).
    list_shown: bool,
    card: Rect,
}

/// The list a dialog shows: its list, a confirmation's listing, or a
/// field's candidates (an empty list while they are closed).
fn list_of(view: &DialogView) -> Option<ListView> {
    match &view.control {
        ControlView::Field(field) => Some(field.candidates.clone().unwrap_or_else(|| ListView {
            name: "Candidates".to_owned(),
            first: 0,
            rows: Vec::new(),
            total: 0,
            focus: None,
        })),
        ControlView::List(list) => Some(list.clone()),
        ControlView::Confirm(confirm) => confirm.listing.clone(),
    }
}

/// Whether the dialog's list is shown: always for a list or a listing,
/// while the candidates are open for a field.
fn list_shown(view: &DialogView) -> bool {
    match &view.control {
        ControlView::Field(field) => field.candidates.is_some(),
        ControlView::List(_) => true,
        ControlView::Confirm(confirm) => confirm.listing.is_some(),
    }
}

impl Modal {
    /// The modal for `view`.
    pub fn new(view: DialogView) -> Self {
        let mut lines = Flex::column();
        for line in &view.lines {
            lines = lines.with_fixed(NewWidget::new(Label::new(line.clone())));
        }
        let list = list_of(&view).map(|list| {
            NewWidget::new(ChoiceBox::new(list))
                .with_tag(DIALOG_LIST)
                .to_pod()
        });
        let field = match &view.control {
            ControlView::Field(field) => {
                let controls: Option<NodeId> = list.as_ref().map(|pod| pod.id().into());
                Some(
                    NewWidget::new(Field::new(field.clone(), controls))
                        .with_tag(DIALOG_FIELD)
                        .to_pod(),
                )
            }
            _ => None,
        };
        let buttons = view
            .buttons
            .iter()
            .map(|button| NewWidget::new(PushButton::new(button.clone())).to_pod())
            .collect();
        Self {
            list_shown: list_shown(&view),
            view,
            title: None,
            lines: NewWidget::new(lines).to_pod(),
            field,
            list,
            buttons,
            card: Rect::ZERO,
        }
    }

    /// What the modal shows.
    pub fn view(&self) -> &DialogView {
        &self.view
    }

    /// The widget the keyboard focus belongs on: the field, the list (a
    /// list shape, a listing, or a field's candidates while the focus is
    /// in them), or a button.
    pub fn focus_id(&self) -> Option<WidgetId> {
        match self.view.focus {
            DialogFocus::Control => {
                let in_list = match &self.view.control {
                    ControlView::Field(field) => field.in_list && field.candidates.is_some(),
                    ControlView::List(_) | ControlView::Confirm(_) => true,
                };
                if in_list {
                    self.list.as_ref().map(WidgetPod::id)
                } else {
                    self.field.as_ref().map(WidgetPod::id)
                }
            }
            DialogFocus::Listing => self.list.as_ref().map(WidgetPod::id),
            other => self
                .buttons
                .iter()
                .zip(&self.view.buttons)
                .find(|(_, view)| view.id == other)
                .map(|(pod, _)| pod.id()),
        }
    }

    /// Shows the same dialog as it is now (a typed character, a moved
    /// focus, candidates listed, a toggle turned): the controls are
    /// changed in place, so their nodes and the focus stay.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: DialogView) {
        let shown = list_shown(&view);
        if let (ControlView::Field(field), Some(pod)) = (&view.control, this.widget.field.as_mut())
        {
            let mut widget = this.ctx.get_mut(pod);
            Field::set_view(&mut widget, field.clone());
        }
        let list = match &view.control {
            // Closed candidates keep the last list hidden.
            ControlView::Field(field) => field.candidates.clone(),
            _ => list_of(&view),
        };
        if let (Some(list), Some(pod)) = (list, this.widget.list.as_mut()) {
            let mut widget = this.ctx.get_mut(pod);
            ChoiceBox::set_view(&mut widget, list);
        }
        for (pod, button) in this.widget.buttons.iter_mut().zip(&view.buttons) {
            let mut widget = this.ctx.get_mut(pod);
            PushButton::set_view(&mut widget, button.clone());
        }
        if shown != this.widget.list_shown {
            this.widget.list_shown = shown;
            this.ctx.request_layout();
        }
        if this.widget.view.title != view.title {
            this.widget.title = None;
            this.ctx.request_layout();
        }
        this.widget.view = view;
        this.ctx.request_render();
        this.ctx.request_accessibility_update();
    }
}

impl Widget for Modal {
    type Action = NoAction;

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        // Clicks on the dimmed page do nothing, and go no further.
        if matches!(event, PointerEvent::Down(..) | PointerEvent::Up(..)) {
            ctx.set_handled();
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.lines);
        if let Some(field) = &mut self.field {
            ctx.register_child(field);
        }
        if let Some(list) = &mut self.list {
            ctx.register_child(list);
        }
        for button in &mut self.buttons {
            ctx.register_child(button);
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
        if self.title.is_none() {
            let mut title = paint::line_layout(ctx, &self.view.title, TITLE_SIZE, true);
            // The title wraps inside the card: a long question is read
            // whole on screen too.
            title.break_all_lines(Some(inner as f32));
            self.title = Some(title);
        }
        let title_h = self
            .title
            .as_ref()
            .map_or(0.0, |layout| f64::from(layout.height()));
        let x0 = ((size.width - width) / 2.0).max(0.0);
        let top = MARGIN;
        let mut y = PAD + title_h + GAP;
        // The lines take the height their text needs (a column asked to
        // fit the window's height would take all of it).
        let lines_h = if self.view.lines.is_empty() {
            0.0
        } else {
            ctx.compute_length(
                &mut self.lines,
                LenDef::MaxContent,
                context,
                Axis::Vertical,
                Some(Length::px(inner)),
            )
            .get()
            .min(size.height / 3.0)
        };
        ctx.run_layout(&mut self.lines, Size::new(inner, lines_h));
        ctx.place_child(&mut self.lines, Point::new(x0 + PAD, top + y));
        y += lines_h + if lines_h > 0.0 { GAP } else { 0.0 };
        if let Some(field) = &mut self.field {
            let h = super::field::FIELD_HEIGHT;
            ctx.run_layout(field, Size::new(inner, h));
            ctx.place_child(field, Point::new(x0 + PAD, top + y));
            y += h + GAP;
        }
        let buttons_h = if self.buttons.is_empty() {
            0.0
        } else {
            super::push::HEIGHT + GAP
        };
        if let Some(list) = &mut self.list {
            ctx.set_stashed(list, !self.list_shown);
            if self.list_shown {
                let room = (size.height - top - y - PAD - MARGIN - buttons_h).max(28.0);
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
                y += h + GAP;
            }
        }
        // The buttons in a row under the rest, No first.
        let mut x = x0 + PAD;
        for button in &mut self.buttons {
            let w = ctx
                .compute_length(
                    button,
                    LenDef::MaxContent,
                    LayoutSize::NONE,
                    Axis::Horizontal,
                    None,
                )
                .get()
                .min(inner);
            ctx.run_layout(button, Size::new(w, super::push::HEIGHT));
            ctx.place_child(button, Point::new(x, top + y));
            x += w + GAP;
        }
        if !self.buttons.is_empty() {
            y += super::push::HEIGHT;
        }
        self.card = Rect::new(x0, top, x0 + width, top + y + PAD);
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let palette = paint::palette_of(props, ctx.property_cache());
        let card = palette.look(diroxide_theme::PanelRole::Dialog);
        let size = ctx.content_box().size();
        // The window behind, veiled in its own page color.
        painter
            .fill(size.to_rect(), paint::color(palette.page).with_alpha(VEIL))
            .draw();
        painter.fill(self.card, paint::color(card.bg)).draw();
        painter
            .stroke(self.card, &Stroke::new(2.0), paint::color(palette.ring))
            .draw();
        if let Some(title) = &self.title {
            render_text(
                painter,
                Affine::translate(Vec2::new(self.card.x0 + PAD, self.card.y0 + PAD)),
                title,
                &[paint::color(card.fg).into()],
                true,
            );
        }
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
        if let Some(description) = &self.view.description {
            node.set_description(description.as_str());
        }
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
        for button in &self.buttons {
            ids.push(button.id());
        }
        ids
    }
}
