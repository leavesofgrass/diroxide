//! Resume through the local provider's own kept write (R25 commit 69):
//! a download from a fake host cut by a drop goes on at its offset into a
//! real folder under the target directory, with no network, and leaves no
//! part file behind.

use std::path::Path;
use std::sync::{Arc, Mutex};

use diroxide_core::{ConflictAnswer, ErrorAction, IdSource, JobKind, JobRequest, JobState, Place};
use diroxide_ops::{
    Conflict, Engine, EngineOptions, FakePacer, FakeProvider, ItemError, JobControl, JobNotice,
    JobProgress, JobSink, LinkDrop,
};
use diroxide_vfs::{LocalProvider, Registry};

struct BringBack {
    fake: FakeProvider,
    host: Place,
    notices: Mutex<Vec<JobNotice>>,
}

impl JobSink for BringBack {
    fn progress(&self, progress: &JobProgress) {
        if progress.state == JobState::WaitingForConnection {
            self.fake.bring_back(&self.host);
        }
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }

    fn notice(&self, notice: &JobNotice) {
        self.notices.lock().unwrap().push(notice.clone());
    }
}

fn place(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

#[test]
fn a_cut_download_resumes_into_a_local_folder() {
    let scratch = tempfile::Builder::new()
        .prefix("resume-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let fake = FakeProvider::new();
    let host = Place::parse("sftp://build/").unwrap();
    let source = Place::parse("sftp://build/srv/big.bin").unwrap();
    let data: Vec<u8> = (0..3_000_000u32)
        .map(|i| (i % 253) as u8 ^ (i >> 9) as u8)
        .collect();
    fake.add_file(&source, &data);
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host.clone(), 1_250_000));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(LocalProvider::new()));
    registry.register("sftp", Arc::new(fake.clone()));
    let options = {
        let mut options = EngineOptions::default();
        options.resume_over = 1_000_000;
        options.chunk_bytes = 64 * 1024;
        options
    };
    let engine = Engine::new(Arc::new(registry))
        .with_options(options)
        .with_pacer(Arc::new(FakePacer::new()));
    let ids = IdSource::new();
    let request = JobRequest::new(ids.job(), JobKind::Copy, vec![source]).to(place(scratch.path()));
    let plan = engine.plan(&request).unwrap();
    let sink = BringBack {
        fake: fake.clone(),
        host,
        notices: Mutex::new(Vec::new()),
    };
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert_eq!(report.done.len(), 1, "{}", report.summary());
    assert_eq!(std::fs::read(scratch.path().join("big.bin")).unwrap(), data);
    let notices = sink.notices.lock().unwrap();
    assert!(
        matches!(
            notices.as_slice(),
            [JobNotice::Resuming { at: 1_250_000, .. }]
        ),
        "{notices:?}"
    );
    let left: Vec<_> = std::fs::read_dir(scratch.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left.len(), 1, "no part file left: {left:?}");
    assert!(engine.ledger().lines().is_empty());
}
