//! Integration tests of the GUI frontend, as one test program (each test
//! program links Masonry, Vello and wgpu again, so new tests are modules of
//! this one, as in textweaver's `tests/it`).

mod contract;
#[cfg(feature = "gui")]
mod dialogs;
#[cfg(feature = "gui")]
mod live_region;
#[cfg(feature = "gui")]
mod loop_keys;
#[cfg(feature = "gui")]
mod menus;
#[cfg(feature = "gui")]
mod panel_list;
#[cfg(feature = "gui")]
mod rig;
#[cfg(feature = "gui")]
mod window_tree;
