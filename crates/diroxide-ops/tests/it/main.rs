//! Integration tests of the job engine: the contract, the journal and the
//! kill test, and the property tests over the fake provider. Tests that
//! write use a folder under the target directory (`CARGO_TARGET_TMPDIR`),
//! never a real user folder, trash or state folder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod interop;
mod journal;
mod kill;
mod links;
mod parts;
mod props;
mod support;

use std::sync::Arc;

use diroxide_core::{IdSource, JobKind, JobRequest, Place};
use diroxide_ops::{Engine, OpsError};
use diroxide_vfs::Registry;

#[test]
fn a_move_plan_lists_every_step_before_anything_runs() {
    let ids = IdSource::new();
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let request = JobRequest::new(ids.job(), JobKind::Move, vec![here.join("src").unwrap()])
        .to(here.join("tests").unwrap());
    let engine = Engine::new(Arc::new(Registry::with_local()));
    let plan = engine.plan(&request).unwrap();
    assert_eq!(plan.listing().len(), 1, "a move on one drive is one rename");
    assert!(plan.listing()[0].starts_with("move "));
    assert!(plan.summary().starts_with("Move 1 item to "));
    let missing = request.clone().to(here.join("elsewhere").unwrap());
    assert!(
        engine.plan(&missing).is_err(),
        "the destination must be there"
    );
    let pack = JobRequest::new(ids.job(), JobKind::Pack, vec![here]);
    assert_eq!(
        Engine::new(Arc::new(Registry::new())).plan(&pack),
        Err(OpsError::NotBuilt("pack".into()))
    );
}
