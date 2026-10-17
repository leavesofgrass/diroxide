//! The join plan (plan section 5.4; research/alpha2-plan.md commit 25): the
//! set found from any part, or given part by part; its sidecar read; what
//! is wrong with it named before anything is written; one step per part.

use std::io::Read;

use diroxide_core::names::split_part;
use diroxide_core::{Entry, EntryKind, Place};
use diroxide_vfs::{Provider, VfsError};

use super::Planner;
use crate::error::OpsError;
use crate::join::JoinFacts;
use crate::part_names::{Discovery, FoundPart, PartSet, Width, find_set};
use crate::plan::{Preflight, Step};
use crate::sidecar::{SidecarRead, sidecar_names};

/// The most of a sidecar that is read: a real one is under 300 bytes.
const SIDECAR_CAP: u64 = 64 * 1024;

impl Planner<'_> {
    /// Join files: the parts of one set, in order, into one file in the
    /// destination. One source is any part, and the set is found from it;
    /// several sources are the parts themselves (the app's answer to
    /// [`Preflight::WhichParts`]).
    pub(super) fn join(&mut self) -> Result<(), OpsError> {
        let folder = self.destination()?;
        self.folder(&folder)?;
        let request = self.request;
        let first = request.sources.first().ok_or(OpsError::NothingSelected)?;
        let parts_folder = first.parent().ok_or(OpsError::NothingSelected)?;
        let provider = self.provider(first)?;
        let case_sensitive = provider.capabilities(&parts_folder).case_sensitive;
        let entries = provider.list(&parts_folder, &self.ctx)?;
        let mut notes = Vec::new();
        let mut set = if request.sources.len() == 1 {
            let name = Self::name_of(first)?;
            match find_set(&parts_folder, &entries, &name, case_sensitive) {
                Err(refusal) => return Err(OpsError::CannotJoin(refusal.to_string())),
                Ok(Discovery::Found(set)) => set,
                Ok(Discovery::Ask { question, yes, no }) => {
                    // The checksum file's size settles which set it is.
                    let size = self
                        .find_sidecar(
                            &*provider,
                            &parts_folder,
                            &entries,
                            &no.base,
                            case_sensitive,
                        )
                        .and_then(|(_, read)| read.size);
                    match size {
                        Some(size) if yes.total() == size => *yes,
                        Some(size) if no.total() == size => *no,
                        _ => {
                            self.plan.facts.questions.push(Preflight::WhichParts {
                                question,
                                yes: yes.places(),
                            });
                            *no
                        }
                    }
                }
            }
        } else {
            given_set(&parts_folder, &entries, &request.sources)?
        };
        notes.append(&mut set.notes);
        let sidecar = self.find_sidecar(
            &*provider,
            &parts_folder,
            &entries,
            &set.base,
            case_sensitive,
        );
        let sidecar = match sidecar {
            Some((place, read)) if read.matches(&set.base) => {
                notes.extend(read.notes());
                Some((place, read))
            }
            Some((place, _)) => {
                notes.push(format!(
                    "Checksum file {} is for another file: not used.",
                    place.name().unwrap_or_default()
                ));
                None
            }
            None => None,
        };
        let size = sidecar.as_ref().and_then(|(_, read)| read.size);
        let problems = set.problems(size);
        // With the checksum file's size, the sum decides: no going on.
        if let Some(problem) = problems
            .iter()
            .find(|problem| size.is_some() || !problem.can_continue())
        {
            return Err(OpsError::CannotJoin(set.sentence(problem, &problems)));
        }
        if let Some(problem) = problems.first() {
            let part = set
                .parts
                .iter()
                .find(|part| matches!(problem, crate::SetProblem::WrongSize { number, .. } if *number == part.number))
                .map_or_else(|| first.clone(), |part| part.place.clone());
            self.plan.facts.questions.push(Preflight::OddPart {
                place: part,
                line: set.sentence(problem, &problems),
            });
        }
        let name = match (&request.new_name, &sidecar) {
            (Some(name), _) => name.clone(),
            (None, Some((_, read))) => read
                .filename
                .clone()
                .filter(|name| plain_name(name))
                .unwrap_or_else(|| set.base.clone()),
            (None, None) => set.base.clone(),
        };
        self.check_new_name(&folder, &name)?;
        let output = folder.join(&name).map_err(VfsError::from)?;
        if set.parts.iter().any(|part| part.place == output) {
            return Err(OpsError::IntoItself(name));
        }
        for part in &set.parts {
            let entry = entry_of(part);
            self.plan.facts.count(&part.place, &entry);
            self.plan.expected.insert(part.place.clone(), entry);
            self.plan.steps.push(Step::JoinPart {
                from: part.place.clone(),
                to: output.clone(),
            });
        }
        let destination = self.provider(&folder)?;
        let mut space = None;
        if let Ok(info) = destination.space(&folder, &self.ctx) {
            let drive = destination
                .volume(&folder, &self.ctx)
                .map_or_else(|_| folder.clone(), |volume| volume.root);
            let total = set.total();
            if let Some(free) = info.free_to_user {
                if total > free {
                    self.plan.facts.questions.push(Preflight::NotEnoughRoom {
                        drive: drive.clone(),
                        needed: total,
                        free,
                    });
                }
                space = Some((drive, free));
            }
            if let Some(max) = info.max_file_size
                && total > max
            {
                self.plan.facts.questions.push(Preflight::TooLarge {
                    place: output.clone(),
                    size: total,
                    max,
                });
            }
        }
        self.plan.facts.counted = true;
        self.plan.facts.destination = Some(folder);
        self.plan.facts.join = Some(JoinFacts {
            set,
            output,
            sidecar,
            notes,
            space,
        });
        Ok(())
    }

    /// The set's sidecar, read: by the names of [`sidecar_names`], in the
    /// parts' folder. A sidecar that cannot be read is no sidecar.
    fn find_sidecar(
        &self,
        provider: &dyn Provider,
        folder: &Place,
        entries: &[Entry],
        base: &str,
        case_sensitive: bool,
    ) -> Option<(Place, SidecarRead)> {
        let fold = |text: &str| {
            if case_sensitive {
                text.to_owned()
            } else {
                text.to_lowercase()
            }
        };
        sidecar_names(base).into_iter().find_map(|wanted| {
            let entry = entries.iter().find(|entry| {
                entry.kind == EntryKind::File && fold(&entry.name) == fold(&wanted)
            })?;
            let place = folder.join(&entry.name).ok()?;
            let mut bytes = Vec::new();
            provider
                .read(&place, &self.ctx)
                .ok()?
                .take(SIDECAR_CAP)
                .read_to_end(&mut bytes)
                .ok()?;
            Some((place, SidecarRead::parse(&bytes)))
        })
    }
}

/// Whether a sidecar's file name can be used as the whole's name: one
/// name, no folder in it.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', '\0']) && name != "." && name != ".."
}

/// A part's entry as the listing saw it, for the plan's re-check.
fn entry_of(part: &FoundPart) -> Entry {
    let mut entry =
        Entry::new(part.place.name().unwrap_or_default(), EntryKind::File).with_size(part.size);
    entry.modified = part.modified;
    entry
}

/// The set the sources name, part by part: each a numbered part of one
/// base, found in the listing.
fn given_set(folder: &Place, entries: &[Entry], sources: &[Place]) -> Result<PartSet, OpsError> {
    let mut parts = Vec::with_capacity(sources.len());
    let mut base: Option<String> = None;
    let mut width = Width::Plain;
    for source in sources {
        let name = source.name().ok_or(OpsError::NothingSelected)?;
        let numbered = split_part(name).ok_or_else(|| {
            OpsError::CannotJoin(
                crate::part_names::NotJoinable::NotAPart(name.to_owned()).to_string(),
            )
        })?;
        match &base {
            Some(known) if known != numbered.base => {
                return Err(OpsError::CannotJoin(format!(
                    "Parts of two files: {known} and {}. Nothing was joined.",
                    numbered.base
                )));
            }
            Some(_) => {}
            None => {
                base = Some(numbered.base.to_owned());
                if numbered.width > numbered.number.to_string().len() && numbered.width > 3 {
                    width = Width::Padded(numbered.width);
                }
            }
        }
        let entry = entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or(OpsError::Vfs(VfsError::NotFound))?;
        parts.push(FoundPart {
            number: numbered.number,
            place: source.clone(),
            size: entry.size.unwrap_or(0),
            modified: entry.modified,
        });
    }
    parts.sort_by_key(|part| part.number);
    let first = if parts.first().is_some_and(|part| part.number == 0) {
        0
    } else {
        1
    };
    Ok(PartSet {
        folder: folder.clone(),
        base: base.unwrap_or_default(),
        width,
        parts,
        first,
        notes: Vec::new(),
    })
}
