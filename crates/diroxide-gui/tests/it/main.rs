//! Integration tests of the GUI frontend, as one test program (each test
//! program links Masonry, Vello and wgpu again, so new tests are modules of
//! this one, as in textweaver's `tests/it`).

mod contract;
#[cfg(feature = "gui")]
mod window_tree;
