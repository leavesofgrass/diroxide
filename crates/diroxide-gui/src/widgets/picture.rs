//! The picture view's widgets (alpha 4, R30 commit 112): the graphic
//! and the lines under it, in the panels' place while a picture is open
//! ([`super::screen::Screen`]).
//!
//! - [`PictureBox`] is one focusable node with the image role, named by
//!   the file and described by the preview crate's sentence, so a screen
//!   reader says "photo.png, graphic, PNG, 1920 by 1080, 8-bit RGBA, 2.3
//!   MB" as the focus moves to it (spike IMG1: UI Automation gives the
//!   node as an Image with that name and FullDescription). It draws the
//!   picture at its zoom, the whole of it at the fit, centered on the
//!   view's middle when zoomed; it tells the driver its size, which the
//!   fit and the zoom's words are measured against.
//! - [`PictureLines`] draws the zoom in words, the animation line, and
//!   the help lines, each one plain text node, so a screen reader's
//!   review reads them too; the frontend says the help line once as the
//!   view opens.
//!
//! Keys are the app's and the window's, through the root, as for every
//! other widget of this crate.

use std::any::TypeId;

use masonry::accesskit::{Node, NodeId, Rect as NodeRect, Role};
use masonry::core::{
    AccessCtx, BrushIndex, ChildrenIds, LayoutCtx, MeasureCtx, NoAction, PaintCtx, PropertiesMut,
    PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;
use masonry::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat};

use super::paint;
use crate::picture::{PictureView, Zoom};

/// What the graphic tells the driver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PictureAction {
    /// The view's size in logical pixels: the fit and the zoom's words
    /// are measured against it.
    Size(f64, f64),
}

/// The graphic.
#[derive(Default)]
pub struct PictureBox {
    view: Option<PictureView>,
    brush: Option<(u64, ImageBrush)>,
    focused: bool,
    told: (f64, f64),
}

impl PictureBox {
    /// An empty graphic, before a picture opens.
    pub fn new() -> Self {
        Self::default()
    }

    /// What it shows.
    pub fn view(&self) -> Option<&PictureView> {
        self.view.as_ref()
    }

    /// Shows `view`, or nothing.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: Option<PictureView>) {
        if this.widget.view == view {
            return;
        }
        let renamed = this.widget.view.as_ref().map(|v| (&v.name, &v.description))
            != view.as_ref().map(|v| (&v.name, &v.description));
        this.widget.view = view;
        this.ctx.request_render();
        if renamed {
            this.ctx.request_accessibility_update();
        }
    }

    /// The brush for the view's pixels, made once a picture.
    fn brush(&mut self) -> Option<&ImageBrush> {
        let view = self.view.as_ref()?;
        if self
            .brush
            .as_ref()
            .is_none_or(|(g, _)| *g != view.generation)
        {
            let data = ImageData {
                data: Blob::new(view.pixels.rgba.clone()),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: view.pixels.width,
                height: view.pixels.height,
            };
            self.brush = Some((view.generation, ImageBrush::new(data)));
        }
        self.brush.as_ref().map(|(_, b)| b)
    }
}

impl Widget for PictureBox {
    type Action = PictureAction;

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::FocusChanged(focused) = event {
            self.focused = *focused;
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
            (Axis::Vertical, _) => Length::px(480.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let told = (size.width, size.height);
        if told != self.told {
            self.told = told;
            ctx.submit_action::<PictureAction>(PictureAction::Size(size.width, size.height));
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
        let area = ctx.content_box();
        let size = area.size();
        painter
            .fill(size.to_rect(), paint::color(palette.page))
            .draw();
        let placed = self.view.as_ref().map(|view| {
            let picture = (view.pixels.width, view.pixels.height);
            let scale = view.zoom.scale((size.width, size.height), picture);
            let center = match view.zoom {
                Zoom::Fit => crate::picture::middle(picture),
                Zoom::Scale(_) => view.center,
            };
            Affine::translate(Vec2::new(size.width / 2.0, size.height / 2.0))
                * Affine::scale(scale)
                * Affine::translate(Vec2::new(-center.0, -center.1))
        });
        if let (Some(transform), Some(brush)) = (placed, self.brush().cloned()) {
            painter.with_fill_clip(area, |painter| {
                painter.draw_image(&brush, transform);
            });
        }
        if self.focused {
            // The focus's shape: the window's ring round the view.
            painter
                .stroke(
                    size.to_rect().inset(-1.5),
                    &Stroke::new(3.0),
                    paint::color(palette.ring),
                )
                .draw();
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Image
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        match &self.view {
            Some(view) => {
                node.set_label(view.name.as_str());
                node.set_description(view.description.as_str());
            }
            None => node.set_label("Picture"),
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

/// The height of one line under the graphic.
pub const LINE_HEIGHT: f64 = 26.0;
const PAD: f64 = 8.0;

/// The lines under the graphic: each drawn, and each a text node.
#[derive(Default)]
pub struct PictureLines {
    lines: Vec<String>,
    layouts: Vec<Layout<BrushIndex>>,
    ids: Vec<NodeId>,
    width: f64,
}

impl PictureLines {
    /// No lines yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The lines shown.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Shows `lines`.
    pub fn set_lines(this: &mut WidgetMut<'_, Self>, lines: Vec<String>) {
        if this.widget.lines == lines {
            return;
        }
        this.widget.lines = lines;
        this.widget.layouts.clear();
        this.ctx.request_layout();
        this.ctx.request_render();
        this.ctx.request_accessibility_update();
    }
}

impl Widget for PictureLines {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn property_changed(&mut self, ctx: &mut UpdateCtx<'_>, property_type: TypeId) {
        if paint::is_colors(property_type) {
            self.layouts.clear();
            ctx.request_layout();
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
            (Axis::Vertical, _) => Length::px(LINE_HEIGHT * self.lines.len().max(1) as f64),
            (Axis::Horizontal, LenReq::FitContent(space)) => space,
            (Axis::Horizontal, _) => Length::px(640.0),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.width = size.width;
        if self.layouts.len() != self.lines.len() {
            self.layouts = self
                .lines
                .iter()
                .map(|line| paint::line_layout(ctx, line, paint::TEXT_SIZE, false))
                .collect();
        }
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let palette = paint::palette_of(props, ctx.property_cache());
        let size = ctx.content_box().size();
        painter
            .fill(size.to_rect(), paint::color(palette.page))
            .draw();
        for (index, layout) in self.layouts.iter().enumerate() {
            let y = index as f64 * LINE_HEIGHT + (LINE_HEIGHT - f64::from(layout.height())) / 2.0;
            render_text(
                painter,
                Affine::translate(Vec2::new(PAD, y)),
                layout,
                &[paint::color(palette.text).into()],
                true,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        while self.ids.len() < self.lines.len() {
            self.ids.push(AccessCtx::next_node_id());
        }
        let mut children = Vec::with_capacity(self.lines.len());
        for (index, line) in self.lines.iter().enumerate() {
            let id = self.ids[index];
            let mut text = Node::new(Role::Label);
            text.set_value(line.as_str());
            let y = index as f64 * LINE_HEIGHT;
            text.set_bounds(NodeRect::new(0.0, y, self.width, y + LINE_HEIGHT));
            ctx.tree_update().nodes.push((id, text));
            children.push(id);
        }
        node.set_children(children);
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
