// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The integration tests of `diroxide-theme`, as one test program.
//!
//! Each module was a file of its own in `tests/`. Every test program
//! links the crate and its dependencies again, so they are kept as
//! one. Add a new integration test as a module in this folder,
//! never as a new file directly in `tests/`.

// The helpers beside the tests fail the test they serve, as a test does.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod builtin;
mod css;
mod props;
mod registry;
mod terminal;
