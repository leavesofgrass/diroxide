//! Lists that are not folders, their types in the app (alpha 5, lane
//! LA): the flat view, collections and recent files, over the library
//! halves in `diroxide_search` (the flat view, collections) and
//! `diroxide_config` (recent files). Each opens in the general results
//! lister (`App::open_list` with a `ListKind`).
//!
//! Reserved by the pre-wiring step (lane P5): [`ListsAsk`] travels as
//! [`Request::Lists`](crate::Request::Lists) to the runtime, which
//! answers in `runtime/lists.rs`, and [`ListsNote`] comes back as
//! [`AppEvent::Lists`](crate::AppEvent::Lists), handled in
//! `app/collections.rs`. Both are empty until lane LA adds its variants
//! here; no shared file changes for them.

/// What the app asks the runtime for these lists. Lane LA adds the
/// variants.
#[derive(Clone, Debug, PartialEq)]
pub enum ListsAsk {}

/// What the runtime answers a [`ListsAsk`]. Lane LA adds the variants.
#[derive(Debug)]
pub enum ListsNote {}
