//! The split plan (plan section 5.4; research/alpha2-plan.md commit 23):
//! each source file's parts and its sidecar as steps, the outcome in words
//! before anything is written, the room checked, the questions asked.

use std::collections::HashSet;

use diroxide_core::names::split_part;
use diroxide_core::{ConflictPolicy, EntryKind};
use diroxide_vfs::VfsError;
use diroxide_vfs::names::numbered_name;

use super::Planner;
use crate::error::OpsError;
use crate::part_names::part_name;
use crate::plan::{Preflight, Step};
use crate::sidecar::SIDECAR_EXTENSION;
use crate::split::{MOST_PARTS, PARTS_WITHOUT_ASKING, SplitFacts, SplitFile};

/// The bytes a sidecar takes, at most, for the room check.
const SIDECAR_BYTES: u64 = 512;

impl Planner<'_> {
    /// Split file: for each source, its parts in the destination, then its
    /// sidecar. Folders, links and empty files are left out with their
    /// reason; a name whose parts are there already gets a free one
    /// ("report (2).pdf"), or is left out under the Skip policy.
    pub(super) fn split(&mut self) -> Result<(), OpsError> {
        let rule = self.request.parts.ok_or(OpsError::NoPartSize)?;
        let folder = self.destination()?;
        self.folder(&folder)?;
        let destination = self.provider(&folder)?;
        let capabilities = destination.capabilities(&folder);
        let sensitive = capabilities.case_sensitive;
        let fold = move |name: &str| {
            if sensitive {
                name.to_owned()
            } else {
                name.to_lowercase()
            }
        };
        // What the destination holds: every name, and every base that has
        // numbered parts, so a new set never mixes with an old one.
        let mut taken: HashSet<String> = HashSet::new();
        let mut bases: HashSet<String> = HashSet::new();
        for entry in destination.list(&folder, &self.ctx)? {
            if let Some(part) = split_part(&entry.name) {
                bases.insert(fold(part.base));
            }
            taken.insert(fold(&entry.name));
        }
        let mut facts = SplitFacts::default();
        let request = self.request;
        for source in &request.sources {
            let name = Self::name_of(source)?;
            let provider = self.provider(source)?;
            let entry = provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            let reason = match entry.kind {
                EntryKind::File | EntryKind::Archive => None,
                EntryKind::Folder => Some("a folder is not split; split the files inside it"),
                _ => Some("only a file can be split"),
            };
            if let Some(reason) = reason {
                self.plan
                    .facts
                    .left_out
                    .push((source.clone(), reason.to_owned()));
                continue;
            }
            let size = entry.size.unwrap_or(0);
            let Some((part_bytes, parts, last_bytes)) = SplitFile::sizes(size, rule) else {
                if size == 0 {
                    self.plan
                        .facts
                        .left_out
                        .push((source.clone(), "it is empty: nothing to split".to_owned()));
                    continue;
                }
                return Err(OpsError::NoPartSize);
            };
            if parts > MOST_PARTS {
                return Err(OpsError::TooManyParts(SplitFacts::too_many(size, parts)));
            }
            let free = |base: &str| {
                !bases.contains(&fold(base))
                    && !taken.contains(&fold(&format!("{base}.{SIDECAR_EXTENSION}")))
            };
            let base = if free(&name) {
                name.clone()
            } else if request.conflicts == ConflictPolicy::Skip {
                self.plan
                    .facts
                    .left_out
                    .push((source.clone(), "its parts are already there".to_owned()));
                continue;
            } else {
                match (2..=999).map(|n| numbered_name(&name, n)).find(|b| free(b)) {
                    Some(base) => base,
                    None => {
                        self.plan.facts.left_out.push((
                            source.clone(),
                            "no free name for its parts after 999 tries".to_owned(),
                        ));
                        continue;
                    }
                }
            };
            let sidecar_name = format!("{base}.{SIDECAR_EXTENSION}");
            // The longest name is the last part's; a name the place refuses
            // is refused before anything is listed, as any create is.
            self.check_new_name(&folder, &part_name(&base, parts))?;
            self.check_new_name(&folder, &sidecar_name)?;
            bases.insert(fold(&base));
            taken.insert(fold(&sidecar_name));
            let sidecar = folder.join(&sidecar_name).map_err(VfsError::from)?;
            for index in 0..parts {
                let to = folder
                    .join(&part_name(&base, index + 1))
                    .map_err(VfsError::from)?;
                let length = if index + 1 == parts {
                    last_bytes
                } else {
                    part_bytes
                };
                self.plan.steps.push(Step::SplitPart {
                    from: source.clone(),
                    to,
                    offset: index * part_bytes,
                    length,
                });
            }
            self.plan.steps.push(Step::WriteChecksum {
                of: source.clone(),
                place: sidecar.clone(),
            });
            if parts > PARTS_WITHOUT_ASKING {
                self.plan.facts.questions.push(Preflight::ManyParts {
                    place: source.clone(),
                    parts,
                });
            }
            self.plan.facts.count(source, &entry);
            facts.files.push(SplitFile {
                source: source.clone(),
                size,
                modified: entry.modified,
                base,
                folder: folder.clone(),
                part_bytes,
                parts,
                last_bytes,
                sidecar,
            });
            self.plan.expected.insert(source.clone(), entry);
        }
        if let Ok(space) = destination.space(&folder, &self.ctx) {
            let needed = facts.bytes() + SIDECAR_BYTES * facts.files.len() as u64;
            let drive = destination
                .volume(&folder, &self.ctx)
                .map_or_else(|_| folder.clone(), |volume| volume.root);
            if let Some(free) = space.free_to_user {
                if needed > free {
                    self.plan.facts.questions.push(Preflight::NotEnoughRoom {
                        drive: drive.clone(),
                        needed,
                        free,
                    });
                }
                facts.space = Some((drive, free));
            }
            if let Some(max) = space.max_file_size {
                for file in facts.files.iter().filter(|file| file.part_bytes > max) {
                    self.plan.facts.questions.push(Preflight::TooLarge {
                        place: file.source.clone(),
                        size: file.part_bytes,
                        max,
                    });
                }
            }
        }
        self.plan.facts.counted = true;
        self.plan.facts.destination = Some(folder);
        self.plan.facts.split = Some(facts);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeProvider;
    use crate::testing::{engine, p};
    use diroxide_core::format::within_cells;
    use diroxide_core::job::PartSize;
    use diroxide_core::{JobId, JobKind, JobRequest};

    fn split(sources: &[&str], size: PartSize) -> JobRequest {
        JobRequest::new(
            JobId::new(20),
            JobKind::Split,
            sources.iter().map(|s| p(s)).collect(),
        )
        .to(p("/b"))
        .split_into(size)
    }

    fn fake_with(size: usize) -> FakeProvider {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/report.pdf"), &vec![7u8; size]);
        fake.add_folder(&p("/b"));
        fake
    }

    fn drive() -> &'static str {
        if cfg!(windows) { "D:" } else { "/" }
    }

    #[test]
    fn the_plan_says_the_parts_before_anything_is_written() {
        let fake = fake_with(223);
        fake.switches_mut().free_space = Some(5_000);
        let plan = engine(&fake)
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(20)))
            .unwrap();
        assert_eq!(plan.summary(), "Split report.pdf into 12 parts, 223 bytes");
        assert_eq!(
            plan.question().text,
            "Split report.pdf into 12 parts, 223 bytes? y or n"
        );
        let facts = plan.facts.split.clone().unwrap();
        assert_eq!(
            facts.rows(),
            vec![
                "12 parts of 20 bytes, the last 3 bytes".to_owned(),
                "223 bytes in all".to_owned(),
                format!("Enough space: 5 KB free on {}", drive()),
            ]
        );
        assert_eq!(plan.steps.len(), 13);
        assert_eq!(
            plan.steps[0],
            Step::SplitPart {
                from: p("/a/report.pdf"),
                to: p("/b/report.pdf.001"),
                offset: 0,
                length: 20
            }
        );
        assert_eq!(
            plan.steps[11],
            Step::SplitPart {
                from: p("/a/report.pdf"),
                to: p("/b/report.pdf.012"),
                offset: 220,
                length: 3
            }
        );
        assert_eq!(
            plan.steps[12],
            Step::WriteChecksum {
                of: p("/a/report.pdf"),
                place: p("/b/report.pdf.crc")
            },
            "the sidecar last"
        );
        assert!(plan.facts.questions.is_empty());
        assert!(fake.unfinished().is_empty() && !fake.exists(&p("/b/report.pdf.001")));
        assert!(plan.listing()[0].starts_with("write the part "));
    }

    #[test]
    fn an_exact_multiple_makes_no_empty_last_part() {
        let fake = fake_with(240);
        let plan = engine(&fake)
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(20)))
            .unwrap();
        assert_eq!(plan.steps.len(), 13);
        assert!(matches!(
            plan.steps[11],
            Step::SplitPart {
                length: 20,
                offset: 220,
                ..
            }
        ));
        assert_eq!(
            plan.facts.split.unwrap().rows()[0],
            "12 parts of 20 bytes each"
        );
        let equal = engine(&fake_with(1000))
            .plan(&split(&["/a/report.pdf"], PartSize::EqualParts(4)))
            .unwrap();
        assert_eq!(
            equal.facts.split.unwrap().rows()[0],
            "4 parts of 250 bytes each"
        );
    }

    #[test]
    fn past_999_parts_asks_and_past_99999_refuses_with_the_size_to_use() {
        let fake = fake_with(1_500);
        let plan = engine(&fake)
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(1)))
            .unwrap();
        let lines = plan.facts.questions[0].lines();
        assert_eq!(lines[0], "This makes 1,500 parts. Go on? y or n");
        assert!(within_cells(&lines[0], "This makes 1,500 parts."));
        assert_eq!(plan.steps.len(), 1_501);
        assert!(matches!(
            &plan.steps[1_499],
            Step::SplitPart { to, .. } if to.name() == Some("report.pdf.1500")
        ));
        let mut answered = plan.clone();
        answered.leave_out(&p("/a/report.pdf"), "skipped by the user");
        assert!(answered.steps.is_empty(), "No leaves the file out");
        let refused = engine(&fake_with(100_000))
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(1)))
            .unwrap_err();
        assert_eq!(
            refused,
            OpsError::TooManyParts("Too many parts: 100,000. Use parts of 2 bytes or more.".into())
        );
    }

    #[test]
    fn too_little_room_asks_and_says_so_in_the_rows() {
        let fake = fake_with(223);
        fake.switches_mut().free_space = Some(100);
        let plan = engine(&fake)
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(20)))
            .unwrap();
        assert!(
            plan.facts.questions[0].lines()[0].starts_with("Not enough room on"),
            "{:?}",
            plan.facts.questions
        );
        assert_eq!(
            plan.facts.split.unwrap().rows()[2],
            format!("Not enough space: 100 bytes free on {}", drive())
        );
    }

    #[test]
    fn parts_of_that_name_there_already_get_a_free_name() {
        let fake = fake_with(50);
        fake.add_file(&p("/b/report.pdf.003"), b"old");
        let plan = engine(&fake)
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(20)))
            .unwrap();
        assert!(matches!(
            &plan.steps[0],
            Step::SplitPart { to, .. } if to.name() == Some("report (2).pdf.001")
        ));
        let rows = plan.facts.split.clone().unwrap().rows();
        assert_eq!(rows[1], "Name taken: parts are report (2).pdf.001 and on");
        fake.add_file(&p("/b/report (2).pdf.crc"), b"old");
        let next = engine(&fake)
            .plan(&split(&["/a/report.pdf"], PartSize::Bytes(20)))
            .unwrap();
        assert!(matches!(
            &next.steps[0],
            Step::SplitPart { to, .. } if to.name() == Some("report (3).pdf.001")
        ));
        let mut skip = split(&["/a/report.pdf"], PartSize::Bytes(20));
        skip.conflicts = ConflictPolicy::Skip;
        let skipped = engine(&fake).plan(&skip).unwrap();
        assert!(skipped.steps.is_empty());
        assert_eq!(skipped.facts.left_out[0].1, "its parts are already there");
    }

    #[test]
    fn only_files_with_bytes_are_split() {
        let fake = fake_with(10);
        fake.add_file(&p("/a/empty.txt"), b"");
        fake.add_folder(&p("/a/photos"));
        let plan = engine(&fake)
            .plan(&split(
                &["/a/empty.txt", "/a/photos", "/a/report.pdf"],
                PartSize::Bytes(4),
            ))
            .unwrap();
        let reasons: Vec<&str> = plan
            .facts
            .left_out
            .iter()
            .map(|(_, r)| r.as_str())
            .collect();
        assert_eq!(
            reasons,
            vec![
                "it is empty: nothing to split",
                "a folder is not split; split the files inside it"
            ]
        );
        assert_eq!(plan.summary(), "Split report.pdf into 3 parts, 10 bytes");
        let engine = engine(&fake);
        assert_eq!(
            engine.plan(&split(&["/a/report.pdf"], PartSize::Bytes(0))),
            Err(OpsError::NoPartSize)
        );
        let mut no_size = split(&["/a/report.pdf"], PartSize::Bytes(4));
        no_size.parts = None;
        assert_eq!(engine.plan(&no_size), Err(OpsError::NoPartSize));
        let mut nowhere = split(&["/a/report.pdf"], PartSize::Bytes(4));
        nowhere.destination = None;
        assert_eq!(engine.plan(&nowhere), Err(OpsError::NoDestination));
    }

    #[test]
    fn several_files_say_each_name_first() {
        let fake = fake_with(10);
        fake.add_file(&p("/a/notes.txt"), &[1u8; 8]);
        let plan = engine(&fake)
            .plan(&split(
                &["/a/report.pdf", "/a/notes.txt"],
                PartSize::Bytes(4),
            ))
            .unwrap();
        assert_eq!(plan.summary(), "Split 2 files into 5 parts, 18 bytes");
        let rows = plan.facts.split.unwrap().rows();
        assert_eq!(rows[0], "report.pdf: 3 parts of 4 bytes, the last 2 bytes");
        assert_eq!(rows[1], "notes.txt: 2 parts of 4 bytes each");
    }
}
