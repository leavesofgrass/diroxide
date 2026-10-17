//! Integration tests of the compare crate.

// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod edges;
mod grammar;
mod properties;
mod sounds_same;
