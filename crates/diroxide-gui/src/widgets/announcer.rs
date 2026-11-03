//! The live region: an invisible widget whose children are the latest
//! messages, each a new AccessKit node, so every message raises UI
//! Automation's LiveRegionChanged and AT-SPI's Announcement, even when the
//! same words come twice ("At the top already" pressed again).
//!
//! - The app's importance decides the politeness: Normal is polite, High
//!   assertive. Low is only shown (the status line), never sent here.
//! - Messages said before a screen reader has asked for the window's tree
//!   (the start's "Opened" lines) are held out of the first tree, where a
//!   live region is not announced, and said once in the next pass
//!   (textweaver's ADR-0028, the status update of Tuesday, September 29,
//!   2026). A full rebuild of the tree later (a screen reader that starts
//!   again) drops the old messages instead of saying them again.
//!
//! Adapted from textweaver's `crates/textweaver-xilem/src/widgets.rs`,
//! `Announcer` (the owner's own project).

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use diroxide_core::{Importance, Message};
use masonry::accesskit::{Live, Node, NodeId, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NoAction, PaintCtx, PropertiesRef, RegisterCtx,
    Widget, WidgetMut,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Size};
use masonry::layout::{LenReq, Length};

/// How many recent messages stay in the tree. A screen reader reads a
/// live region's change when it happens; older messages stay only so a
/// quick burst is not cut short by the next update.
pub const KEEP: usize = 3;

/// How many messages wait for a screen reader to ask for the tree; older
/// ones are dropped (no screen reader may ever ask).
pub const HOLD: usize = 12;

/// The politeness a message is announced with, or `None` when it is only
/// shown.
pub fn live_for(importance: Importance) -> Option<Live> {
    match importance {
        Importance::Low => None,
        Importance::Normal => Some(Live::Polite),
        Importance::High => Some(Live::Assertive),
    }
}

/// The live region.
pub struct Announcer {
    pending: VecDeque<(String, Live)>,
    shown: VecDeque<(NodeId, String, Live)>,
    full_passes: Rc<Cell<u64>>,
    seen_full: u64,
    tree_seen: bool,
    release_due: bool,
    announced: usize,
}

impl Announcer {
    /// An announcer with nothing to say. `full_passes` is the root's count
    /// of passes that rebuild every node.
    pub fn new(full_passes: Rc<Cell<u64>>) -> Self {
        let seen_full = full_passes.get();
        Self {
            pending: VecDeque::new(),
            shown: VecDeque::new(),
            full_passes,
            seen_full,
            tree_seen: false,
            release_due: false,
            announced: 0,
        }
    }

    /// Queues the messages that are said (Normal and High) for the next
    /// accessibility pass; Low ones are skipped.
    pub fn say(this: &mut WidgetMut<'_, Self>, messages: impl IntoIterator<Item = Message>) {
        let before = this.widget.pending.len();
        for message in messages {
            if let Some(live) = live_for(message.importance) {
                this.widget.pending.push_back((message.text, live));
            }
        }
        if !this.widget.tree_seen {
            while this.widget.pending.len() > HOLD {
                this.widget.pending.pop_front();
            }
        }
        if this.widget.pending.len() != before {
            this.ctx.request_accessibility_update();
        }
    }

    /// Says the messages held until a screen reader asked for the tree, in
    /// the next pass, as new nodes after the tree.
    pub fn release(this: &mut WidgetMut<'_, Self>) {
        if std::mem::take(&mut this.widget.release_due) {
            this.ctx.request_accessibility_update();
        }
    }

    /// Whether held messages wait to be released (the driver then calls
    /// [`Self::release`] on its next turn).
    pub fn release_due(&self) -> bool {
        self.release_due
    }

    /// True while messages wait for a screen reader to ask for the tree,
    /// or to be released after it did: the driver looks again soon.
    pub fn holding(&self) -> bool {
        self.release_due || (!self.tree_seen && !self.pending.is_empty())
    }

    /// True once a screen reader has asked for the tree.
    pub fn tree_seen(&self) -> bool {
        self.tree_seen
    }

    /// The messages in the tree now, oldest first, with their politeness.
    pub fn shown(&self) -> impl Iterator<Item = (&str, Live)> {
        self.shown
            .iter()
            .map(|(_, text, live)| (text.as_str(), *live))
    }

    /// How many messages were announced in all.
    pub fn announced(&self) -> usize {
        self.announced
    }
}

impl Widget for Announcer {
    type Action = NoAction;

    fn accepts_pointer_interaction(&self) -> bool {
        false
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        _len_req: LenReq,
        _cross: Option<Length>,
    ) -> Length {
        Length::ZERO
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, _size: Size) {}

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _p: &mut Painter<'_>) {
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
        if self.full_passes.get() != self.seen_full {
            self.seen_full = self.full_passes.get();
            self.shown.clear();
            if !self.tree_seen {
                // The first tree a screen reader gets: what was said before
                // it asked waits for the next pass, where it is new.
                self.tree_seen = true;
                if !self.pending.is_empty() {
                    self.release_due = true;
                    node.set_children(Vec::<NodeId>::new());
                    return;
                }
            }
        }
        while let Some((text, live)) = self.pending.pop_front() {
            self.shown
                .push_back((AccessCtx::next_node_id(), text, live));
            self.announced += 1;
            while self.shown.len() > KEEP {
                self.shown.pop_front();
            }
        }
        for (id, text, live) in &self.shown {
            // A label's name is its value in AccessKit.
            let mut message = Node::new(Role::Label);
            message.set_value(text.as_str());
            message.set_live(*live);
            ctx.tree_update().nodes.push((*id, message));
        }
        node.set_children(self.shown.iter().map(|(id, _, _)| *id).collect::<Vec<_>>());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
