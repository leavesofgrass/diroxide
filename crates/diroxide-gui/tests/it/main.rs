//! Integration tests of the GUI frontend, as one test program (each test
//! program links Masonry, Vello and wgpu again, so new tests are modules of
//! this one, as in textweaver's `tests/it`).

#[cfg(feature = "gui")]
mod choose;
#[cfg(feature = "gui")]
mod completion;
#[cfg(feature = "gui")]
mod confirm;
#[cfg(feature = "gui")]
mod connect;
mod contract;
#[cfg(feature = "gui")]
mod dialogs;
#[cfg(feature = "gui")]
mod drawing;
#[cfg(feature = "gui")]
mod every_purpose;
#[cfg(feature = "gui")]
mod goldens;
#[cfg(feature = "gui")]
mod gui_keys;
#[cfg(feature = "gui")]
mod handover;
#[cfg(feature = "gui")]
mod jobs;
#[cfg(feature = "gui")]
mod keybar_mouse;
#[cfg(feature = "gui")]
mod keys;
#[cfg(feature = "gui")]
mod layouts;
#[cfg(feature = "gui")]
mod live_region;
#[cfg(feature = "gui")]
mod loop_keys;
#[cfg(feature = "gui")]
mod menus;
#[cfg(feature = "gui")]
mod panel_list;
#[cfg(feature = "gui")]
mod plugins;
#[cfg(feature = "gui")]
mod quick_view;
#[cfg(feature = "gui")]
mod rig;
#[cfg(feature = "gui")]
mod secret;
#[cfg(feature = "gui")]
mod ten_minutes;
#[cfg(feature = "gui")]
mod themes;
#[cfg(feature = "gui")]
mod viewer;
#[cfg(feature = "gui")]
mod window_tree;
