//! A one-line text field driven by the app's entry model (spike S7: the
//! orchestrator's decision, app-driven for every field, Masonry's
//! `TextInput` bridged in only if this failed for plain text).
//!
//! The app holds the text and the caret ([`diroxide_app::entry::EntryField`]);
//! every key goes to the app through the window's root, and the field
//! shows what the app has: its label, its text and its caret. For a
//! screen reader the field is a text input whose text is one text run,
//! with each character's length and the words' lengths, and whose caret
//! is the text selection, so UI Automation's TextPattern and AT-SPI's
//! Text interface read the characters as the caret moves over them.
//!
//! A field with completion is an editable combo box: expanded while its
//! candidates are listed, and naming the list it controls. A password
//! field gets only the mask the app draws (one asterisk a character, or
//! nothing): the secret never reaches the field, the drawing or the
//! tree, unless the owner turns the dialog's Show toggle on.
//!
//! Every field but a password's opens an input method session when it
//! takes the focus, its candidate window at the caret; what the input
//! method commits, and a paste, reach the app as typed text through the
//! window's root.

use masonry::accesskit::{Node, NodeId, Rect as NodeRect, Role, TextPosition, TextSelection};
use masonry::core::{
    AccessCtx, BrushIndex, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx, PointerEvent,
    PropertiesMut, PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::Layout;

use super::paint;
use crate::model::FieldView;

/// The field's height in logical pixels.
pub const FIELD_HEIGHT: f64 = 34.0;
const PAD: f64 = 8.0;

/// The UTF-8 length of each character of `text`, as AccessKit's text runs
/// count them.
pub fn character_lengths(text: &str) -> Vec<u8> {
    text.chars().map(|c| c.len_utf8() as u8).collect()
}

/// Where each word starts, in characters: the first character, and each
/// one that follows a space (a word keeps the spaces after it, as
/// AccessKit's text runs define words). AccessKit counts them in a byte,
/// so a field's words past its 255th character are not marked.
pub fn word_starts(text: &str) -> Vec<u8> {
    let mut starts = Vec::new();
    let mut after_space = true;
    for (i, c) in text.chars().enumerate() {
        let space = c.is_whitespace();
        if !space && after_space {
            match u8::try_from(i) {
                Ok(i) => starts.push(i),
                Err(_) => break,
            }
        }
        after_space = space;
    }
    starts
}

/// What a field tells the driver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldAction {
    /// A click on the field: the app keeps its caret, and the focus comes
    /// back to the field.
    Clicked,
}

/// A one-line field showing the app's entry.
pub struct Field {
    view: FieldView,
    /// The node the candidates list has, for the combo box's "controls".
    list: Option<NodeId>,
    run: Option<NodeId>,
    focused: bool,
    text: Option<Layout<BrushIndex>>,
    before_caret: Option<Layout<BrushIndex>>,
    width: f64,
    /// Whether the field opens an input method session when it takes the
    /// focus: every field but a password's (Masonry reads it once, when
    /// the widget is made).
    input_method: bool,
}

impl Field {
    /// A field showing `view`; `list` is the candidates list's node.
    pub fn new(view: FieldView, list: Option<NodeId>) -> Self {
        Self {
            input_method: !view.secret,
            view,
            list,
            run: None,
            focused: false,
            text: None,
            before_caret: None,
            width: 0.0,
        }
    }

    /// What the field shows.
    pub fn view(&self) -> &FieldView {
        &self.view
    }

    /// Shows the app's entry as it is now.
    pub fn set_view(this: &mut WidgetMut<'_, Self>, view: FieldView) {
        if this.widget.view == view {
            return;
        }
        if this.widget.view.text != view.text || this.widget.view.cursor != view.cursor {
            this.widget.text = None;
            this.widget.before_caret = None;
            this.ctx.request_layout();
        }
        this.widget.view = view;
        this.ctx.request_render();
    }

    /// The text as drawn and given to the tree.
    fn shown(&self) -> &str {
        &self.view.text
    }

    fn role(&self) -> Role {
        if self.view.secret {
            Role::PasswordInput
        } else if self.view.combo {
            Role::EditableComboBox
        } else {
            Role::TextInput
        }
    }
}

impl Widget for Field {
    type Action = FieldAction;

    fn accepts_focus(&self) -> bool {
        true
    }

    fn accepts_text_input(&self) -> bool {
        // An input method composes text for the field (Chinese, Japanese,
        // Korean, dictation); what it commits reaches the app as typed
        // text through the window's root. A password field opens none.
        self.input_method
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::FocusChanged(focused) = event {
            self.focused = *focused;
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
            ctx.submit_action::<FieldAction>(FieldAction::Clicked);
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
            (Axis::Horizontal, LenReq::FitContent(space)) => space,
            (Axis::Horizontal, _) => Length::px(480.0),
            (Axis::Vertical, _) => Length::px(FIELD_HEIGHT),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.width = size.width;
        if self.text.is_none() {
            let text = self.shown().to_owned();
            self.text = Some(paint::line_layout(ctx, &text, paint::TEXT_SIZE, false));
            let before: String = text.chars().take(self.view.cursor).collect();
            self.before_caret = Some(paint::line_layout(ctx, &before, paint::TEXT_SIZE, false));
        }
        // The input method's candidate window goes at the caret.
        let x = PAD
            + self
                .before_caret
                .as_ref()
                .map_or(0.0, |l| f64::from(l.width()));
        ctx.set_ime_area(Rect::new(x, 0.0, x + 2.0, size.height));
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let size = ctx.content_box().size();
        let frame = Rect::new(1.0, 1.0, size.width - 1.0, size.height - 1.0);
        painter.fill(frame, paint::PAGE).draw();
        let ring = if self.focused { 3.0 } else { 1.0 };
        painter
            .stroke(frame, &Stroke::new(ring), paint::RING)
            .draw();
        let Some(text) = &self.text else {
            return;
        };
        let y = (size.height - f64::from(text.height())) / 2.0;
        render_text(
            painter,
            Affine::translate(Vec2::new(PAD, y)),
            text,
            &[paint::TEXT.into()],
            true,
        );
        // The caret: a bar two pixels wide, so it shows without color.
        let x = PAD
            + self
                .before_caret
                .as_ref()
                .map_or(0.0, |l| f64::from(l.width()));
        painter
            .fill(Rect::new(x, 6.0, x + 2.0, size.height - 6.0), paint::TEXT)
            .draw();
    }

    fn accessibility_role(&self) -> Role {
        self.role()
    }

    fn accessibility(
        &mut self,
        ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_label(self.view.label.as_str());
        let text = self.shown().to_owned();
        node.set_value(text.as_str());
        let run_id = *self.run.get_or_insert_with(AccessCtx::next_node_id);
        let mut run = Node::new(Role::TextRun);
        run.set_value(text.as_str());
        run.set_character_lengths(character_lengths(&text));
        run.set_word_starts(word_starts(&text));
        run.set_bounds(NodeRect::new(PAD, 0.0, self.width - PAD, FIELD_HEIGHT));
        ctx.tree_update().nodes.push((run_id, run));
        node.set_children(vec![run_id]);
        let caret = TextPosition {
            node: run_id,
            character_index: self.view.cursor.min(text.chars().count()),
        };
        node.set_text_selection(TextSelection {
            anchor: caret,
            focus: caret,
        });
        if self.view.combo {
            match (self.view.candidates.is_some(), self.list) {
                (true, Some(list)) => {
                    node.set_expanded(true);
                    node.set_controls(vec![list]);
                }
                _ => node.set_expanded(false),
            }
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_character_has_its_length_and_every_word_its_start() {
        let text = "Rename to report-2026.pdf ok";
        let chars = character_lengths(text);
        assert_eq!(chars.len(), text.chars().count());
        assert_eq!(word_starts(text), [0, 7, 10, 26]);
        assert_eq!(word_starts("  two spaces"), [2, 6]);
        assert_eq!(character_lengths("a\u{e9}\u{1f99d}"), [1, 2, 4]);
        assert!(word_starts("").is_empty());
        let long = "x ".repeat(200);
        assert!(word_starts(&long).len() < 200, "only starts under 256");
    }
}
