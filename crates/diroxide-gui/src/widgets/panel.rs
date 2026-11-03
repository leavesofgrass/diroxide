//! A panel: one focusable AccessKit list box whose options are the rows
//! the app shows in it ([`PanelView`]), as ADR-0007 decides.
//!
//! - The list box is named by its panel ("Left panel, Documents"); its
//!   header, the title line for the active panel, is drawn above the
//!   rows and is not a second node.
//! - Each row in the window is an option node (not a widget) named by the
//!   row's label and details, with its absolute position; the list
//!   carries the real number of rows and is multiselectable; a marked row
//!   is selected, an unavailable one disabled (on macOS both stay in the
//!   name; see [`crate::model::states_in_name`]).
//! - The focused row is the list's active descendant, so a screen reader
//!   follows the focus with no announcement of its own (textweaver's
//!   ADR-0027).
//! - The list clears the clip flag Masonry sets for its painting, or
//!   AccessKit's filter would leave out the options scrolled out of view
//!   (textweaver's ADR-0028).
//! - Option node ids come from a pool keyed by the row's id, so a row
//!   keeps its node while it stays in the window; another list (another
//!   place) starts the pool again, so its first row is a new node and is
//!   said. Masonry's `VirtualScroll` is not used: its accessibility is
//!   not designed yet (R26 section 5).
//! - Keys are the app's: the list takes the focus and leaves every key to
//!   the window's root, which hands it to the app. A click focuses the
//!   panel and its row; a double click opens the row; the wheel scrolls.
//!   A screen reader's Focus, Click or ScrollIntoView on an option moves
//!   the app's focus to that row.
//!
//! Adapted from textweaver's `crates/textweaver-xilem/src/dialog.rs`,
//! `ChoiceList` (the owner's own project).

use std::collections::HashMap;

use diroxide_core::Side;
use masonry::accesskit::{Action, Node, NodeId, Rect as NodeRect, Role};
use masonry::core::{
    AccessCtx, AccessEvent, BrushIndex, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerButtonEvent, PointerEvent, PointerScrollEvent, PropertiesMut, PropertiesRef,
    RegisterCtx, Update, UpdateCtx, Widget, WidgetMut, render_text,
};
use masonry::dpi::{LogicalPosition, PhysicalPosition};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;

use super::paint;
use crate::model::PanelView;

/// What a panel tells the driver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAction {
    /// The panel took the keyboard focus (a click, a screen reader's
    /// focus): the app makes it the active panel.
    Focused(Side),
    /// How many rows the panel shows now: Page Up and Page Down move this
    /// many, and the window of nodes is built around them.
    Page(Side, usize),
    /// The wheel scrolled the rows; the first row drawn is now this one.
    Scrolled(Side, usize),
    /// A click, or a screen reader's action, put the focus on the row
    /// with this id.
    FocusRow(Side, String),
    /// A double click opened the row with this id.
    Open(Side, String),
}

/// The pool of option node ids, keyed by the rows' ids.
#[derive(Debug, Default)]
struct NodePool {
    generation: u64,
    ids: HashMap<String, NodeId>,
}

impl NodePool {
    /// The node id of a row, made when the row first enters the window.
    fn id(&mut self, row: &str) -> NodeId {
        *self
            .ids
            .entry(row.to_owned())
            .or_insert_with(AccessCtx::next_node_id)
    }

    /// Starts again for another list, and forgets rows long gone from
    /// the window, so the pool stays near the window's size.
    fn keep(&mut self, generation: u64, window: &[String]) {
        if generation != self.generation {
            self.generation = generation;
            self.ids.clear();
            return;
        }
        if self.ids.len() > window.len().max(64) * 4 {
            self.ids.retain(|id, _| window.contains(id));
        }
    }
}

/// The row height in logical pixels.
pub const ROW_HEIGHT: f64 = 28.0;
/// The header's height.
pub const HEADER_HEIGHT: f64 = 30.0;
const PAD: f64 = 8.0;

/// A panel's list box.
pub struct PanelBox {
    view: PanelView,
    focused: bool,
    visible: usize,
    width: f64,
    header: Option<Layout<BrushIndex>>,
    /// The drawn rows' layouts, by absolute index, with their text.
    rows: HashMap<usize, (String, Layout<BrushIndex>)>,
    empty: Option<Layout<BrushIndex>>,
    pool: NodePool,
    /// The empty list's one node.
    empty_id: Option<NodeId>,
    /// The page last told to the driver.
    told_page: usize,
}

impl PanelBox {
    /// A panel showing `view`.
    pub fn new(view: PanelView) -> Self {
        Self {
            view,
            focused: false,
            visible: 0,
            width: 0.0,
            header: None,
            rows: HashMap::new(),
            empty: None,
            pool: NodePool::default(),
            empty_id: None,
            told_page: 0,
        }
    }

    /// What the panel shows.
    pub fn view(&self) -> &PanelView {
        &self.view
    }

    /// How many rows fit, from the last layout.
    pub fn visible_rows(&self) -> usize {
        self.visible
    }

    /// Whether the list has the keyboard focus.
    pub fn has_focus(&self) -> bool {
        self.focused
    }

    /// The node id the focused row has (the active descendant), once the
    /// tree was built.
    pub fn focused_node(&self) -> Option<NodeId> {
        let row = self.view.focused()?;
        self.pool.ids.get(&row.id).copied()
    }

    /// Shows `view`: a new window of rows, the focus, the header.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: PanelView) {
        if this.widget.view == view {
            return;
        }
        let relayout = this.widget.view.header != view.header
            || this.widget.view.top != view.top
            || this.widget.view.generation != view.generation
            || this.widget.view.rows != view.rows
            || this.widget.view.empty != view.empty;
        if this.widget.view.header != view.header {
            this.widget.header = None;
        }
        this.widget.view = view;
        if relayout {
            this.ctx.request_layout();
        }
        this.ctx.request_render();
    }

    fn row_at(&self, y: f64) -> Option<usize> {
        let y = y - HEADER_HEIGHT;
        if y < 0.0 {
            return None;
        }
        let index = self.view.top + (y / ROW_HEIGHT) as usize;
        (index < self.view.total).then_some(index)
    }
}

impl Widget for PanelBox {
    type Action = PanelAction;

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::FocusChanged(focused) = event {
            self.focused = *focused;
            if *focused {
                ctx.submit_action::<PanelAction>(PanelAction::Focused(self.view.side));
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
        match event {
            PointerEvent::Down(PointerButtonEvent { state, .. }) => {
                ctx.request_focus();
                let local = ctx.local_position(state.position);
                if let Some(index) = self.row_at(local.y)
                    && let Some(row) = self.view.row(index)
                {
                    let id = row.id.clone();
                    let side = self.view.side;
                    if state.count >= 2 && self.view.focus == Some(index) {
                        ctx.submit_action::<PanelAction>(PanelAction::Open(side, id));
                    } else {
                        ctx.submit_action::<PanelAction>(PanelAction::FocusRow(side, id));
                    }
                }
                ctx.set_handled();
            }
            PointerEvent::Scroll(PointerScrollEvent { delta, .. }) => {
                let scale = ctx.scale_factor();
                let pixels = delta.to_pixel_delta(
                    PhysicalPosition::new(ROW_HEIGHT * scale, ROW_HEIGHT * scale),
                    PhysicalPosition::new(ROW_HEIGHT * 8.0 * scale, ROW_HEIGHT * 8.0 * scale),
                );
                let LogicalPosition { y, .. } = pixels.to_logical::<f64>(scale);
                let rows = (-y / ROW_HEIGHT).round() as isize;
                let max_top = self.view.total.saturating_sub(self.visible.max(1));
                let top = (self.view.top as isize + rows).clamp(0, max_top as isize) as usize;
                if top != self.view.top {
                    ctx.submit_action::<PanelAction>(PanelAction::Scrolled(self.view.side, top));
                }
                ctx.set_handled();
            }
            _ => {}
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
        let Some(node) = event.node else {
            if event.action == Action::Focus {
                ctx.request_focus();
                ctx.set_handled();
            }
            return;
        };
        let row = self
            .pool
            .ids
            .iter()
            .find(|(_, id)| **id == node)
            .map(|(row, _)| row.clone());
        if let Some(row) = row {
            ctx.request_focus();
            ctx.submit_action::<PanelAction>(PanelAction::FocusRow(self.view.side, row));
            ctx.set_handled();
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
            (Axis::Horizontal, _) => Length::px(320.0),
            (Axis::Vertical, _) => Length::px(HEADER_HEIGHT + ROW_HEIGHT * 12.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let visible = (((size.height - HEADER_HEIGHT) / ROW_HEIGHT).floor() as usize).max(1);
        self.visible = visible;
        if visible != self.told_page {
            self.told_page = visible;
            ctx.submit_action::<PanelAction>(PanelAction::Page(self.view.side, visible));
        }
        if (size.width - self.width).abs() > 0.5 {
            self.width = size.width;
        }
        if self.header.is_none() {
            self.header = Some(paint::line_layout(
                ctx,
                &self.view.header,
                paint::TEXT_SIZE,
                true,
            ));
        }
        let first = self.view.top;
        let last = (first + visible + 1).min(self.view.total);
        self.rows.retain(|index, _| (first..last).contains(index));
        for index in first..last {
            let Some(row) = self.view.row(index) else {
                continue;
            };
            let fresh = self
                .rows
                .get(&index)
                .is_some_and(|(text, _)| *text == row.text);
            if !fresh {
                let text = row.text.clone();
                let layout = paint::line_layout(ctx, &text, paint::TEXT_SIZE, false);
                self.rows.insert(index, (text, layout));
            }
        }
        self.empty = (self.view.total == 0)
            .then(|| paint::line_layout(ctx, &self.view.empty, paint::TEXT_SIZE, false));
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let size = ctx.content_box().size();
        painter.fill(size.to_rect(), paint::PAGE).draw();
        if let Some(header) = &self.header {
            let y = (HEADER_HEIGHT - f64::from(header.height())) / 2.0;
            render_text(
                painter,
                Affine::translate(Vec2::new(PAD, y)),
                header,
                &[paint::HEADER.into()],
                true,
            );
        }
        // A line under the header, so the rows read as the header's.
        painter
            .fill(
                Rect::new(
                    PAD,
                    HEADER_HEIGHT - 2.0,
                    size.width - PAD,
                    HEADER_HEIGHT - 1.0,
                ),
                paint::HEADER,
            )
            .draw();
        if let Some(empty) = &self.empty {
            let y = HEADER_HEIGHT + (ROW_HEIGHT - f64::from(empty.height())) / 2.0;
            render_text(
                painter,
                Affine::translate(Vec2::new(PAD, y)),
                empty,
                &[paint::TEXT.into()],
                true,
            );
        }
        let first = self.view.top;
        let last = (first + self.visible + 1).min(self.view.total);
        for index in first..last {
            let Some(row) = self.view.row(index) else {
                continue;
            };
            let y = HEADER_HEIGHT + (index - first) as f64 * ROW_HEIGHT;
            let bar = Rect::new(2.0, y + 1.0, size.width - 2.0, y + ROW_HEIGHT - 1.0);
            let focused = self.view.focus == Some(index);
            let mut color = if !row.enabled {
                paint::DIM
            } else if row.selected {
                paint::MARKED
            } else {
                paint::TEXT
            };
            if focused {
                if self.view.active {
                    painter.fill(bar, paint::FOCUS_BAR).draw();
                    color = paint::ON_FOCUS_BAR;
                }
                // The outline, in both panels: the focus's shape.
                painter
                    .stroke(bar.inset(-1.0), &Stroke::new(2.0), paint::RING)
                    .draw();
            }
            if let Some((_, layout)) = self.rows.get(&index) {
                let ty = y + (ROW_HEIGHT - f64::from(layout.height())) / 2.0;
                render_text(
                    painter,
                    Affine::translate(Vec2::new(PAD, ty)),
                    layout,
                    &[color.into()],
                    true,
                );
            }
        }
        if self.focused {
            // The ring of the panel with the keyboard focus.
            painter
                .stroke(size.to_rect().inset(-1.5), &Stroke::new(3.0), paint::RING)
                .draw();
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
        // The painting is clipped, but every option in the window is a
        // node with its own bounds, so the list does not claim to clip
        // them (textweaver's ADR-0028: options vanish from the tree
        // otherwise).
        node.clear_clips_children();
        node.set_multiselectable();
        node.set_size_of_set(self.view.total);
        let window: Vec<String> = self.view.rows.iter().map(|r| r.id.clone()).collect();
        self.pool.keep(self.view.generation, &window);
        let in_name = crate::model::states_in_name();
        let mut children = Vec::with_capacity(self.view.rows.len().max(1));
        for (offset, row) in self.view.rows.iter().enumerate() {
            let index = self.view.first + offset;
            let id = self.pool.id(&row.id);
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
            // AccessKit's position is zero-based; the adapters add one.
            option.set_position_in_set(index);
            option.add_action(Action::Focus);
            option.add_action(Action::Click);
            option.add_action(Action::ScrollIntoView);
            let y = HEADER_HEIGHT + (index as f64 - self.view.top as f64) * ROW_HEIGHT;
            option.set_bounds(NodeRect::new(0.0, y, self.width, y + ROW_HEIGHT));
            ctx.tree_update().nodes.push((id, option));
            children.push(id);
        }
        if self.view.total == 0 {
            // An empty list has one line that says why ("Empty folder",
            // "Reading folder"), focused as the terminal's cursor is on
            // it; it is not an option, so it is not counted.
            let id = *self.empty_id.get_or_insert_with(AccessCtx::next_node_id);
            let mut line = Node::new(Role::Label);
            line.set_value(self.view.empty.as_str());
            line.set_bounds(NodeRect::new(
                0.0,
                HEADER_HEIGHT,
                self.width,
                HEADER_HEIGHT + ROW_HEIGHT,
            ));
            ctx.tree_update().nodes.push((id, line));
            children.push(id);
            node.set_active_descendant(id);
        } else if let Some(row) = self.view.focused() {
            let id = self.pool.id(&row.id);
            node.set_active_descendant(id);
        }
        node.set_children(children);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
