//! The viewer: one focusable list box whose options are the rows on
//! screen, text or hex, with the same rules as a panel's (`super::panel`):
//! each option named by its row's text, its position the line's place in
//! the file and the list's size the lines counted so far, the cursor's
//! row the active descendant, the clip flag cleared. The viewer itself
//! gives only the rows on screen, so a file of 100,000 lines is a screen
//! of nodes.
//!
//! - Its size is told to the driver ([`ViewerAction::Size`]): the columns
//!   that fit (from the font's width) and the rows, which the app's
//!   viewer wraps and pages by.
//! - Token classes are drawn with the theme's token roles
//!   ([`super::paint::token_look`]): a role's color where it keeps its
//!   contrast on the window's page, and its shape (bold keywords and
//!   headings, underlined links) always, so a class never shows by color
//!   alone; a screen reader hears classes in words through Say Token
//!   Kind and Say Line by Kind. Search matches are underlined.
//! - Keys are the app's, through the window's root, in the viewer's own
//!   layer; the prompts (jump, search) and the outline are dialogs.

use std::collections::{BTreeMap, HashMap};

use masonry::accesskit::{Action, Node, NodeId, Rect as NodeRect, Role};
use masonry::core::{
    AccessCtx, BrushIndex, ChildrenIds, LayoutCtx, MeasureCtx, PaintCtx, PropertiesMut,
    PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;
use masonry::peniko::Color;

use super::paint::{self, Run};
use crate::model::ViewerView;

/// The row height in logical pixels.
pub const ROW_HEIGHT: f64 = 26.0;
const PAD: f64 = 8.0;

/// What the viewer tells the driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerAction {
    /// The columns and rows that fit now.
    Size(usize, usize),
}

/// The viewer's list box.
pub struct ViewerBox {
    view: ViewerView,
    focused: bool,
    width: f64,
    /// The size last told to the driver.
    told: (usize, usize),
    /// The drawn rows' layouts and colors, by index, with their text.
    rows: HashMap<usize, (String, Layout<BrushIndex>, Vec<Color>)>,
    /// Option node ids by row id, started again for another file or mode.
    ids: HashMap<String, NodeId>,
    generation: u64,
    /// The theme and the owner's colors the token roles are drawn with.
    theme: String,
    colors: BTreeMap<String, String>,
}

impl ViewerBox {
    /// A viewer showing `view` (an empty one before a file opens).
    pub fn new(view: ViewerView) -> Self {
        Self {
            generation: view.generation,
            view,
            focused: false,
            width: 0.0,
            told: (0, 0),
            rows: HashMap::new(),
            ids: HashMap::new(),
            theme: diroxide_theme::DEFAULT_THEME.to_owned(),
            colors: BTreeMap::new(),
        }
    }

    /// What the viewer shows.
    pub fn view(&self) -> &ViewerView {
        &self.view
    }

    /// Shows the rows as the app's viewer has them now.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: ViewerView) {
        if this.widget.view == view {
            return;
        }
        if view.generation != this.widget.generation {
            this.widget.generation = view.generation;
            this.widget.ids.clear();
        }
        this.widget.view = view;
        this.widget.rows.clear();
        this.ctx.request_layout();
        this.ctx.request_render();
        this.ctx.request_accessibility_update();
    }

    /// Draws the token roles with the theme `theme` and the owner's
    /// `[colors]`.
    pub fn set_theme(
        this: &mut WidgetMut<'_, Self>,
        theme: String,
        colors: BTreeMap<String, String>,
    ) {
        if this.widget.theme != theme || this.widget.colors != colors {
            this.widget.theme = theme;
            this.widget.colors = colors;
            this.widget.rows.clear();
            this.ctx.request_layout();
            this.ctx.request_render();
        }
    }

    fn id_of(&mut self, row: &str) -> NodeId {
        *self
            .ids
            .entry(row.to_owned())
            .or_insert_with(AccessCtx::next_node_id)
    }

    /// A row's runs and colors: brush 0 the row's own color, then one a
    /// token run, then the matches' underline.
    fn row_runs(&self, index: usize, focused: bool) -> (Vec<Run>, Vec<Color>) {
        let Some(row) = self.view.rows.get(index) else {
            return (Vec::new(), vec![paint::TEXT]);
        };
        let base = if focused && self.focused {
            paint::ON_FOCUS_BAR
        } else {
            paint::TEXT
        };
        let mut colors = vec![base];
        let mut runs = Vec::new();
        if !row.runs.is_empty() {
            let registry = diroxide_theme::Registry::builtin();
            let (theme, _) = registry.resolve(&self.theme);
            for (start, end, role) in &row.runs {
                let look = paint::token_look(theme, &self.colors, *role);
                // On the focus bar the text keeps the bar's own color, and
                // its shape.
                let brush = if focused && self.focused {
                    0
                } else {
                    colors.push(look.color);
                    colors.len() - 1
                };
                runs.push(Run {
                    start: *start,
                    end: *end,
                    brush,
                    bold: look.bold,
                    underline: look.underline,
                });
            }
        }
        for (start, end) in &row.matches {
            runs.push(Run {
                start: *start,
                end: *end,
                brush: 0,
                bold: true,
                underline: true,
            });
        }
        (runs, colors)
    }
}

impl Widget for ViewerBox {
    type Action = ViewerAction;

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::FocusChanged(focused) = event {
            self.focused = *focused;
            self.rows.clear();
            ctx.request_layout();
            ctx.request_render();
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
            (Axis::Vertical, _) => Length::px(ROW_HEIGHT * 20.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.width = size.width;
        // The columns that fit: the interface font's width of a typical
        // character, measured once a layout.
        let probe = paint::line_layout(ctx, "0123456789", paint::TEXT_SIZE, false);
        let char_width = (f64::from(probe.width()) / 10.0).max(1.0);
        let columns = (((size.width - 2.0 * PAD) / char_width).floor() as usize).max(10);
        let rows = ((size.height / ROW_HEIGHT).floor() as usize).max(1);
        if (columns, rows) != self.told {
            self.told = (columns, rows);
            ctx.submit_action::<ViewerAction>(ViewerAction::Size(columns, rows));
        }
        for index in 0..self.view.rows.len() {
            let focused = self.view.focus == Some(index);
            let text = self.view.rows[index].text.clone();
            if self.rows.get(&index).is_some_and(|(t, _, _)| *t == text) {
                continue;
            }
            let (runs, colors) = self.row_runs(index, focused);
            let layout = paint::runs_layout(ctx, &text, paint::TEXT_SIZE, &runs);
            self.rows.insert(index, (text, layout, colors));
        }
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
        for index in 0..self.view.rows.len() {
            let y = index as f64 * ROW_HEIGHT;
            if y > size.height {
                break;
            }
            let bar = Rect::new(2.0, y + 1.0, size.width - 2.0, y + ROW_HEIGHT - 1.0);
            if self.view.focus == Some(index) {
                if self.focused {
                    painter.fill(bar, paint::FOCUS_BAR).draw();
                }
                painter
                    .stroke(bar.inset(-1.0), &Stroke::new(2.0), paint::RING)
                    .draw();
            }
            if let Some((_, layout, colors)) = self.rows.get(&index) {
                let ty = y + (ROW_HEIGHT - f64::from(layout.height())) / 2.0;
                let brushes: Vec<_> = colors.iter().map(|c| (*c).into()).collect();
                render_text(
                    painter,
                    Affine::translate(Vec2::new(PAD, ty)),
                    layout,
                    &brushes,
                    true,
                );
            }
        }
        if self.focused {
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
        node.clear_clips_children();
        let size = usize::try_from(self.view.lines).unwrap_or(usize::MAX);
        node.set_size_of_set(size);
        let rows = self.view.rows.clone();
        let mut children = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let id = self.id_of(&row.id);
            let mut option = Node::new(Role::ListBoxOption);
            option.set_label(row.name.as_str());
            // AccessKit's position is zero-based; the adapters add one, so
            // a screen reader says the line's own number.
            option.set_position_in_set(usize::try_from(row.line).unwrap_or(usize::MAX));
            option.add_action(Action::Focus);
            option.add_action(Action::ScrollIntoView);
            let y = index as f64 * ROW_HEIGHT;
            option.set_bounds(NodeRect::new(0.0, y, self.width, y + ROW_HEIGHT));
            ctx.tree_update().nodes.push((id, option));
            children.push(id);
        }
        if let Some(row) = self.view.focus.and_then(|i| rows.get(i)) {
            let id = self.id_of(&row.id);
            node.set_active_descendant(id);
        }
        node.set_children(children);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
