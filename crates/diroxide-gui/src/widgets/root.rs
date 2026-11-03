//! The window's content: the root, which takes every key for the app, and
//! the pieces around the panels.
//!
//! - [`Root`]: the main column, the live region, and the dialog over them
//!   (the modal host). Every key that reaches it goes to the driver as a
//!   [`RootAction`]; nothing is left to Masonry, so Tab is the app's
//!   (Switch panel) and Masonry's own F11 and F12 debugging keys never
//!   fire. While a dialog is open, the window behind it is disabled and
//!   hidden from screen readers, so reading and the focus stay inside.
//! - [`FullPassProbe`]: tells the live region when an accessibility pass
//!   rebuilds every node (a screen reader asked for the tree).
//! - [`Region`]: a part of the window with a role and a name (the status
//!   bar), filled with a theme role's band when it has one.
//!
//! Adapted from textweaver's `crates/textweaver-xilem/src/widgets.rs`
//! (`Root`, `FullPassProbe`, `Region`; the owner's own project).

use std::cell::Cell;
use std::rc::Rc;

use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::{Key, KeyState, KeyboardEvent, NamedKey};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, Ime, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx,
    PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Point, Size};
use masonry::layout::{LayoutSize, LenReq, Length};

use super::modal::Modal;
use super::paint;
use crate::model::DialogView;

/// What the root tells the driver.
#[derive(Clone, Debug, PartialEq)]
pub enum RootAction {
    /// A key pressed, for the app.
    Key(KeyboardEvent),
    /// Text an input method finished composing, or pasted text: typed
    /// into the open field.
    Text(String),
    /// Alt was pressed and let go with no other key: the hidden menu bar
    /// shows (with native menus).
    AltAlone,
}

/// The window's content.
pub struct Root {
    /// Alt is down and no other key came since.
    alt_alone: bool,
    probe: WidgetPod<FullPassProbe>,
    main: WidgetPod<Region>,
    announcer: WidgetPod<dyn Widget>,
    dialog: Option<WidgetPod<Modal>>,
    full_passes: Rc<Cell<u64>>,
}

impl Root {
    /// The root over `main` with the live region `announcer`;
    /// `full_passes` is shared with the announcer.
    pub fn new(
        main: NewWidget<Region>,
        announcer: NewWidget<impl Widget + ?Sized>,
        full_passes: Rc<Cell<u64>>,
    ) -> Self {
        Self {
            alt_alone: false,
            probe: NewWidget::new(FullPassProbe {
                full_passes: Rc::clone(&full_passes),
            })
            .to_pod(),
            main: main.to_pod(),
            announcer: announcer.erased().to_pod(),
            dialog: None,
            full_passes,
        }
    }

    /// The count of passes that rebuilt every node.
    pub fn full_passes(&self) -> u64 {
        self.full_passes.get()
    }

    /// Shows the app's top dialog over the window, changing the open one
    /// in place when it is the same dialog in the same shape, or closes
    /// it with `None`. While a dialog is open the window behind it is
    /// disabled and hidden from screen readers.
    pub fn set_dialog(this: &mut WidgetMut<'_, Self>, view: Option<DialogView>) {
        if let (Some(view), Some(pod)) = (&view, this.widget.dialog.as_mut()) {
            let mut modal = this.ctx.get_mut(pod);
            if modal.widget.view().same_shape(view) {
                Modal::set_view(&mut modal, view.clone());
                return;
            }
        }
        if let Some(old) = this.widget.dialog.take() {
            this.ctx.remove_child(old);
        }
        let open = view.is_some();
        this.widget.dialog = view.map(|view| NewWidget::new(Modal::new(view)).to_pod());
        {
            let mut main = this.ctx.get_mut(&mut this.widget.main);
            main.ctx.set_disabled(open);
            Region::set_hidden(&mut main, open);
        }
        this.ctx.children_changed();
        this.ctx.request_layout();
        this.ctx.request_accessibility_update();
    }

    /// Whether a dialog is open.
    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }
}

impl Root {
    /// The widget in the open dialog the keyboard focus belongs on.
    pub fn dialog_focus(this: &mut WidgetMut<'_, Self>) -> Option<masonry::core::WidgetId> {
        let pod = this.widget.dialog.as_mut()?;
        let modal = this.ctx.get_mut(pod);
        modal.widget.focus_id()
    }
}

impl Widget for Root {
    type Action = RootAction;

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        match event {
            TextEvent::Keyboard(key) => {
                let alt = key.key == Key::Named(NamedKey::Alt);
                if key.state == KeyState::Down {
                    // Alt alone is told apart from Alt with a key: any
                    // other key between Alt's press and its release.
                    self.alt_alone = alt;
                    ctx.submit_action::<RootAction>(RootAction::Key(key.clone()));
                } else if alt && std::mem::take(&mut self.alt_alone) {
                    ctx.submit_action::<RootAction>(RootAction::AltAlone);
                }
                // A release runs nothing else; it is still handled, so
                // Masonry never acts on it.
                ctx.set_handled();
            }
            TextEvent::Ime(Ime::Commit(text)) => {
                ctx.submit_action::<RootAction>(RootAction::Text(text.clone()));
                ctx.set_handled();
            }
            TextEvent::ClipboardPaste(text) => {
                ctx.submit_action::<RootAction>(RootAction::Text(text.clone()));
                ctx.set_handled();
            }
            _ => {}
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.probe);
        ctx.register_child(&mut self.main);
        ctx.register_child(&mut self.announcer);
        if let Some(dialog) = &mut self.dialog {
            ctx.register_child(dialog);
        }
    }

    fn measure(
        &mut self,
        ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        _len_req: LenReq,
        cross: Option<Length>,
    ) -> Length {
        ctx.redirect_measurement(&mut self.main, axis, cross)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        ctx.run_layout(&mut self.probe, Size::ZERO);
        ctx.place_child(&mut self.probe, Point::ORIGIN);
        ctx.run_layout(&mut self.main, size);
        ctx.place_child(&mut self.main, Point::ORIGIN);
        ctx.run_layout(&mut self.announcer, Size::ZERO);
        ctx.place_child(&mut self.announcer, Point::ORIGIN);
        if let Some(dialog) = &mut self.dialog {
            ctx.run_layout(dialog, size);
            ctx.place_child(dialog, Point::ORIGIN);
        }
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let palette = paint::palette_of(props, ctx.property_cache());
        painter
            .fill(
                ctx.content_box().size().to_rect(),
                paint::color(palette.page),
            )
            .draw();
    }

    fn property_changed(
        &mut self,
        ctx: &mut masonry::core::UpdateCtx<'_>,
        property_type: std::any::TypeId,
    ) {
        if paint::is_colors(property_type) {
            ctx.request_render();
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        let mut ids =
            ChildrenIds::from_slice(&[self.probe.id(), self.main.id(), self.announcer.id()]);
        if let Some(dialog) = &self.dialog {
            ids.push(dialog.id());
        }
        ids
    }
}

/// Counts the accessibility passes that rebuild every node (a screen
/// reader asked for the tree, or asked again after a restart), for the
/// live region. It is the root's first child: zero-sized at the origin,
/// never laid out again or moved, so Masonry rebuilds its node only when
/// it rebuilds them all.
pub struct FullPassProbe {
    full_passes: Rc<Cell<u64>>,
}

impl Widget for FullPassProbe {
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
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_hidden();
        self.full_passes.set(self.full_passes.get().wrapping_add(1));
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

/// A part of the window with a role and a name.
pub struct Region {
    child: WidgetPod<dyn Widget>,
    role: Role,
    label: String,
    hidden: bool,
    /// The theme role whose band fills the region (the status bar's).
    band: Option<diroxide_theme::PanelRole>,
}

impl Region {
    /// A region around `child`; an empty `label` gives it no name, so a
    /// screen reader reads its children.
    pub fn new(
        child: NewWidget<impl Widget + ?Sized>,
        role: Role,
        label: impl Into<String>,
    ) -> Self {
        Self {
            child: child.erased().to_pod(),
            role,
            label: label.into(),
            hidden: false,
            band: None,
        }
    }

    /// The same region filled with `band`'s background (the status bar
    /// in the status line's band); its labels take the band's text color
    /// from the driver.
    #[must_use]
    pub fn with_band(mut self, band: diroxide_theme::PanelRole) -> Self {
        self.band = Some(band);
        self
    }

    /// Hides the region and everything in it from screen readers (while
    /// a dialog is open over it), or shows it again.
    pub fn set_hidden(this: &mut WidgetMut<'_, Self>, hidden: bool) {
        if this.widget.hidden != hidden {
            this.widget.hidden = hidden;
            this.ctx.request_accessibility_update();
        }
    }

    /// The child, to change it.
    pub fn child_mut<'t>(this: &'t mut WidgetMut<'_, Self>) -> WidgetMut<'t, dyn Widget> {
        this.ctx.get_mut(&mut this.widget.child)
    }
}

impl Widget for Region {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.child);
    }

    fn measure(
        &mut self,
        ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        cross: Option<Length>,
    ) -> Length {
        let context = LayoutSize::maybe(axis.cross(), cross);
        ctx.compute_length(&mut self.child, len_req.into(), context, axis, cross)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        ctx.run_layout(&mut self.child, size);
        ctx.place_child(&mut self.child, Point::ORIGIN);
        ctx.derive_baselines(&self.child);
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, props: &PropertiesRef<'_>, p: &mut Painter<'_>) {
        if let Some(band) = self.band {
            let palette = paint::palette_of(props, ctx.property_cache());
            let look = palette.look(band);
            p.fill(ctx.content_box().size().to_rect(), paint::color(look.bg))
                .draw();
        }
    }

    fn property_changed(
        &mut self,
        ctx: &mut masonry::core::UpdateCtx<'_>,
        property_type: std::any::TypeId,
    ) {
        if self.band.is_some() && paint::is_colors(property_type) {
            ctx.request_render();
        }
    }

    fn accessibility_role(&self) -> Role {
        self.role
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        if !self.label.is_empty() {
            node.set_label(self.label.as_str());
        }
        if self.hidden {
            node.set_hidden();
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.child.id()])
    }
}
