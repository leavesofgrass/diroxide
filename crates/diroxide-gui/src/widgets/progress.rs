//! The progress of the job in front: one progress indicator in the status
//! bar under the jobs line, its value the job's percent, drawn as a bar
//! with the percent in words beside it, so the amount is never shown by
//! a color or a length alone.
//!
//! Its value changes only when the app's jobs line changes, which the app
//! paces itself: a change of state at once, progress at most every
//! `progress_interval_seconds` in screen-reader mode and twice a second
//! otherwise (`crates/diroxide-app/src/jobs.rs`). A screen reader that
//! reports progress bars hears it no more often than the line. It is not
//! a live region, and with no job running it is not in the tree.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, BrushIndex, ChildrenIds, LayoutCtx, MeasureCtx, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;

use super::paint;

/// The indicator's name.
pub const NAME: &str = "Job progress";

const HEIGHT: f64 = 22.0;
const BAR_WIDTH: f64 = 240.0;
const PAD: f64 = 8.0;

/// The job in front's progress.
pub struct JobProgressBar {
    percent: Option<u64>,
    layout: Option<Layout<BrushIndex>>,
}

impl JobProgressBar {
    /// An indicator showing `percent`, or nothing.
    pub fn new(percent: Option<u64>) -> Self {
        Self {
            percent,
            layout: None,
        }
    }

    /// The percent shown, if a job runs.
    pub fn percent(&self) -> Option<u64> {
        self.percent
    }

    /// Shows another percent, or none when no job runs.
    pub fn set_percent(this: &mut WidgetMut<'_, Self>, percent: Option<u64>) {
        let percent = percent.map(|p| p.min(100));
        if this.widget.percent != percent {
            this.widget.percent = percent;
            this.widget.layout = None;
            this.ctx.request_layout();
            this.ctx.request_render();
            this.ctx.request_accessibility_update();
        }
    }

    /// The percent in words: "40 percent".
    pub fn words(percent: u64) -> String {
        format!("{percent} percent")
    }
}

impl Widget for JobProgressBar {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

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
            (Axis::Horizontal, _) => Length::px(BAR_WIDTH + 120.0),
            (Axis::Vertical, _) => {
                if self.percent.is_some() {
                    Length::px(HEIGHT)
                } else {
                    Length::ZERO
                }
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, _size: Size) {
        if self.layout.is_none()
            && let Some(percent) = self.percent
        {
            self.layout = Some(paint::line_layout(
                ctx,
                &Self::words(percent),
                paint::TEXT_SIZE - 2.0,
                false,
            ));
        }
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let Some(percent) = self.percent else {
            return;
        };
        let height = ctx.content_box().size().height;
        let frame = Rect::new(PAD, 4.0, PAD + BAR_WIDTH, height - 4.0);
        painter.stroke(frame, &Stroke::new(1.0), paint::RING).draw();
        let filled = frame.inset(-2.0);
        let width = filled.width() * percent as f64 / 100.0;
        painter
            .fill(
                Rect::new(filled.x0, filled.y0, filled.x0 + width, filled.y1),
                paint::FOCUS_BAR,
            )
            .draw();
        if let Some(layout) = &self.layout {
            let y = (height - f64::from(layout.height())) / 2.0;
            render_text(
                painter,
                Affine::translate(Vec2::new(PAD * 2.0 + BAR_WIDTH, y)),
                layout,
                &[paint::TEXT.into()],
                true,
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::ProgressIndicator
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(NAME);
        match self.percent {
            Some(percent) => {
                node.set_min_numeric_value(0.0);
                node.set_max_numeric_value(100.0);
                node.set_numeric_value(percent as f64);
                node.set_value(Self::words(percent));
            }
            None => node.set_hidden(),
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
