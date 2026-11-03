//! A list inside a dialog: one focusable AccessKit list box over the
//! app's list model ([`ListView`]), with the same rules as a panel's
//! (`super::panel`): options named by the row's label and details, the
//! states as states, absolute positions, the real count, the focus as the
//! active descendant, the clip flag cleared, a window of rows as nodes.
//! The keys are the app's, through the window's root.
//!
//! A click on a row, or a screen reader's Focus or Click on an option,
//! tells the driver the row ([`ChoiceAction::Row`]), which moves the
//! list's focus there (the app's, or the window's own for a text to read);
//! a double click chooses the row, as Enter does.
//!
//! Adapted from textweaver's `crates/textweaver-xilem/src/dialog.rs`,
//! `ChoiceList` (the owner's own project).

use std::any::TypeId;
use std::collections::HashMap;

use diroxide_theme::PanelRole;
use masonry::accesskit::{Action, Node, NodeId, Rect as NodeRect, Role};
use masonry::core::{
    AccessCtx, AccessEvent, BrushIndex, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Update, UpdateCtx,
    Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Rect, Size, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;

use super::paint;
use crate::model::{ListView, scroll_top};

const ROW_HEIGHT: f64 = 28.0;
const PAD: f64 = 8.0;
/// The most rows a dialog's list asks room for.
const MAX_ROWS: usize = 12;

/// What a dialog's list tells the driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceAction {
    /// The list took the keyboard focus another way than the driver's (a
    /// click, a screen reader's Focus): the dialog's focus follows.
    Focused,
    /// A click or a screen reader's action on the row at this index,
    /// counted from the list's start; `open` for a double click, which
    /// chooses it.
    Row {
        /// The row.
        index: usize,
        /// Whether to choose it, as Enter does.
        open: bool,
    },
    /// A question's message list took the focus another way than the
    /// driver's.
    MessageFocused,
    /// A click or a screen reader's action on a line of a question's
    /// message.
    MessageLine(usize),
}

/// A dialog's list box.
pub struct ChoiceBox {
    view: ListView,
    focused: bool,
    top: usize,
    visible: usize,
    width: f64,
    rows: HashMap<usize, (String, Layout<BrushIndex>)>,
    ids: HashMap<String, NodeId>,
    /// A question's message, whose lines are read, never chosen.
    message: bool,
}

impl ChoiceBox {
    /// A list showing `view`.
    pub fn new(view: ListView) -> Self {
        Self {
            view,
            focused: false,
            top: 0,
            visible: 1,
            width: 0.0,
            rows: HashMap::new(),
            ids: HashMap::new(),
            message: false,
        }
    }

    /// A question's message as a list of its lines to read.
    pub fn message(view: ListView) -> Self {
        Self {
            message: true,
            ..Self::new(view)
        }
    }

    /// What a click or a screen reader's action on the row at `index`
    /// tells the driver.
    fn row_action(&self, index: usize, open: bool) -> ChoiceAction {
        if self.message {
            ChoiceAction::MessageLine(index)
        } else {
            ChoiceAction::Row { index, open }
        }
    }

    /// What the list shows.
    pub fn view(&self) -> &ListView {
        &self.view
    }

    /// Shows the list as the app has it now; another list (another name)
    /// starts its node ids again.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: ListView) {
        if this.widget.view == view {
            return;
        }
        if this.widget.view.name != view.name {
            this.widget.ids.clear();
        }
        this.widget.view = view;
        this.ctx.request_layout();
        this.ctx.request_render();
    }

    fn id_of(&mut self, row: &str) -> NodeId {
        *self
            .ids
            .entry(row.to_owned())
            .or_insert_with(AccessCtx::next_node_id)
    }

    /// The row drawn at `y`, from the list's start.
    fn row_at(&self, y: f64) -> Option<usize> {
        if y < 0.0 {
            return None;
        }
        let index = self.top + (y / ROW_HEIGHT) as usize;
        (index < self.view.total).then_some(index)
    }

    /// The row whose option node is `node`, from the list's start.
    fn row_of_node(&self, node: NodeId) -> Option<usize> {
        let id = self
            .ids
            .iter()
            .find(|(_, n)| **n == node)
            .map(|(id, _)| id.as_str())?;
        let offset = self.view.rows.iter().position(|row| row.id == id)?;
        Some(self.view.first + offset)
    }
}

impl Widget for ChoiceBox {
    type Action = ChoiceAction;

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn property_changed(&mut self, ctx: &mut UpdateCtx<'_>, property_type: TypeId) {
        if paint::is_colors(property_type) {
            ctx.request_render();
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::FocusChanged(focused) = event {
            self.focused = *focused;
            if *focused {
                ctx.submit_action::<ChoiceAction>(if self.message {
                    ChoiceAction::MessageFocused
                } else {
                    ChoiceAction::Focused
                });
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
        if let PointerEvent::Down(PointerButtonEvent { state, .. }) = event {
            ctx.request_focus();
            let local = ctx.local_position(state.position);
            if let Some(index) = self.row_at(local.y) {
                let open = state.count >= 2 && self.view.focus == Some(index);
                ctx.submit_action::<ChoiceAction>(self.row_action(index, open));
            }
            ctx.set_handled();
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        if !matches!(
            event.action,
            Action::Focus | Action::Click | Action::ScrollIntoView
        ) {
            return;
        }
        ctx.request_focus();
        if let Some(index) = event.node.and_then(|node| self.row_of_node(node)) {
            ctx.submit_action::<ChoiceAction>(self.row_action(index, false));
        }
        ctx.set_handled();
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
            (Axis::Horizontal, _) => Length::px(480.0),
            (Axis::Vertical, _) => {
                let rows = self.view.total.clamp(1, MAX_ROWS) as f64;
                Length::px(rows * ROW_HEIGHT)
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.width = size.width;
        self.visible = ((size.height / ROW_HEIGHT).floor() as usize).max(1);
        self.top = scroll_top(self.top, self.view.focus, self.visible, self.view.total);
        let last = (self.top + self.visible + 1).min(self.view.total);
        let range = self.top..last;
        self.rows.retain(|index, _| range.contains(index));
        for index in range {
            let Some(row) = self.view.row(index) else {
                continue;
            };
            if self
                .rows
                .get(&index)
                .is_none_or(|(text, _)| *text != row.text)
            {
                let text = row.text.clone();
                let layout = paint::line_layout(ctx, &text, paint::TEXT_SIZE, false);
                self.rows.insert(index, (text, layout));
            }
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let palette = paint::palette_of(props, ctx.property_cache());
        let size = ctx.content_box().size();
        // The list on the page's color inside the dialog's card, so its
        // rows read as a panel's do.
        painter
            .fill(size.to_rect(), paint::color(palette.page))
            .draw();
        let last = (self.top + self.visible + 1).min(self.view.total);
        for index in self.top..last {
            let Some(row) = self.view.row(index) else {
                continue;
            };
            let y = (index - self.top) as f64 * ROW_HEIGHT;
            let bar = Rect::new(2.0, y + 1.0, size.width - 2.0, y + ROW_HEIGHT - 1.0);
            let look = if row.selected {
                palette.look(PanelRole::MarkedRow)
            } else {
                palette.look(row.role)
            };
            let mut color = if row.enabled { look.fg } else { palette.dim };
            if row.enabled && look.bg != palette.page {
                painter.fill(bar, paint::color(look.bg)).draw();
            }
            if self.view.focus == Some(index) {
                if self.focused {
                    let band = palette.look(PanelRole::FocusedRow);
                    painter.fill(bar, paint::color(band.bg)).draw();
                    color = band.fg;
                }
                paint::outline(painter, bar, &palette, self.focused);
            }
            if let Some((_, layout)) = self.rows.get(&index) {
                let ty = y + (ROW_HEIGHT - f64::from(layout.height())) / 2.0;
                render_text(
                    painter,
                    Affine::translate(Vec2::new(PAD, ty)),
                    layout,
                    &[paint::color(color).into()],
                    true,
                );
            }
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::ListBox
    }

    fn accessibility(
        &mut self,
        ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.view.name.as_str());
        node.clear_clips_children();
        node.set_size_of_set(self.view.total);
        let in_name = crate::model::states_in_name();
        let rows = self.view.rows.clone();
        let mut children = Vec::with_capacity(rows.len());
        for (offset, row) in rows.iter().enumerate() {
            let index = self.view.first + offset;
            let id = self.id_of(&row.id);
            let mut option = Node::new(Role::ListBoxOption);
            option.set_label(row.name.as_str());
            if !in_name {
                if row.selected {
                    option.set_selected(true);
                }
                if !row.enabled {
                    option.set_disabled();
                }
            }
            option.set_position_in_set(index);
            option.add_action(Action::Focus);
            option.add_action(Action::Click);
            option.add_action(Action::ScrollIntoView);
            let y = (index as f64 - self.top as f64) * ROW_HEIGHT;
            option.set_bounds(NodeRect::new(0.0, y, self.width, y + ROW_HEIGHT));
            ctx.tree_update().nodes.push((id, option));
            children.push(id);
        }
        if let Some(row) = self.view.focus.and_then(|i| self.view.row(i)).cloned() {
            let id = self.id_of(&row.id);
            node.set_active_descendant(id);
        }
        node.set_children(children);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
