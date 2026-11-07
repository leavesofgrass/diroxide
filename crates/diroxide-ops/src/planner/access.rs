//! The permissions, attributes and times plan (research/permissions.md 6.5
//! to 7; research/alpha2-plan.md commit 27).

use std::collections::BTreeMap;

use diroxide_core::names::spell_out;
use diroxide_core::{Attributes, Entry, EntryKind, JobKind, JobRequest, Place};
use diroxide_vfs::access::{Caller, find_group, find_user};
use diroxide_vfs::{AttributeChange, Principal, Provider};

use super::{Planner, Removal};
use crate::access::{
    AttributeEdit, AttributeFacts, ModeChange, OwnerWords, change_words, group_row, mode_line,
    owner_protection, time_words,
};
use crate::engine::{Engine, JobControl, Quiet};
use crate::error::OpsError;
use crate::plan::{Plan, Preflight, Step};
use crate::walk::Found;

/// An owner change, resolved before the walk (R30's commit 79): the new
/// owner and group as numbers with their names, and who asks.
struct OwnerChange {
    owner: Option<Principal>,
    group: Option<Principal>,
    caller: Caller,
}

/// A principal as said: its name, or its number.
fn shown(principal: &Principal) -> &str {
    principal.name.as_deref().unwrap_or(&principal.id)
}

/// Whether an entry's owner or group, as a listing gives it (a name, or
/// a number when no name is known), is this principal already.
fn same(current: Option<&str>, principal: &Principal) -> bool {
    current.is_some_and(|current| {
        current == principal.id || principal.name.as_deref() == Some(current)
    })
}

/// An owner refusal, said whole with its own next step (AX's request 1):
/// `OpsError::OwnerRefused`, to which the app adds nothing.
fn refused(sentence: String) -> OpsError {
    OpsError::OwnerRefused(sentence)
}

#[cfg(test)]
thread_local! {
    /// The caller the owner plan sees in a test.
    static TEST_CALLER: std::cell::RefCell<Option<Caller>> = const { std::cell::RefCell::new(None) };
}

/// Who asks for the change: the running program, or a test's caller.
fn caller() -> Caller {
    #[cfg(test)]
    if let Some(caller) = TEST_CALLER.with(|slot| slot.borrow().clone()) {
        return caller;
    }
    Caller::current()
}

/// Runs a test's plan as this caller.
#[cfg(test)]
pub(crate) fn as_caller<T>(caller: Caller, then: impl FnOnce() -> T) -> T {
    TEST_CALLER.with(|slot| *slot.borrow_mut() = Some(caller));
    let out = then();
    TEST_CALLER.with(|slot| *slot.borrow_mut() = None);
    out
}

impl OwnerChange {
    /// The words of this change, by the names given.
    fn words(&self) -> OwnerWords<'_> {
        OwnerWords {
            owner: self.owner.as_ref().map(shown),
            group: self.group.as_ref().map(shown),
        }
    }

    /// Resolves the typed owner and group: a name this computer knows, or
    /// a number (an owner can be any number; an SFTP place takes numbers
    /// only). `first` names the first entry, for the refusal.
    fn resolve(edit: &AttributeEdit, first: &str) -> Result<Self, OpsError> {
        let typed = OwnerWords {
            owner: edit.owner.as_deref(),
            group: edit.group.as_deref(),
        };
        let number = |text: &str| {
            text.parse::<u32>().ok().map(|n| Principal {
                id: n.to_string(),
                name: None,
            })
        };
        let owner = match edit.owner.as_deref() {
            Some(text) => Some(
                find_user(text)
                    .or_else(|| number(text))
                    .ok_or_else(|| refused(typed.unknown(first, text, false)))?,
            ),
            None => None,
        };
        let group = match edit.group.as_deref() {
            Some(text) => Some(
                find_group(text)
                    .or_else(|| number(text))
                    .ok_or_else(|| refused(typed.unknown(first, text, true)))?,
            ),
            None => None,
        };
        Ok(Self {
            owner,
            group,
            caller: caller(),
        })
    }

    /// Refuses, before anything is listed, an owner change at a place
    /// that cannot take one (Windows, which hands off to its own dialog; a
    /// place that keeps no owners) or at a protected place.
    fn check_place(
        edit: &AttributeEdit,
        source: &Place,
        provider: &dyn Provider,
    ) -> Result<(), OpsError> {
        let typed = OwnerWords {
            owner: edit.owner.as_deref(),
            group: edit.group.as_deref(),
        };
        let shown_place = spell_out(&source.to_string()).into_owned();
        if let Some(why) = owner_protection(source) {
            return Err(refused(typed.protected(&shown_place, why)));
        }
        let access = provider.capabilities(source).access;
        let possible = (edit.owner.is_none() || access.owner.is_possible())
            && (edit.group.is_none() || access.group.is_possible());
        if !possible {
            let name = source
                .name()
                .map_or(shown_place.clone(), |n| spell_out(n).into_owned());
            let windows = cfg!(windows) && provider.scheme() == "file";
            return Err(refused(typed.not_here(&name, windows)));
        }
        Ok(())
    }

    /// Adds this change to one entry's, where it differs from what the
    /// entry has; refuses what only an administrator could do, and a
    /// group the caller is not in. Returns the plan line's change words.
    fn apply(
        &self,
        place: &Place,
        entry: &Entry,
        change: &mut AttributeChange,
    ) -> Result<Option<String>, OpsError> {
        let owner = self
            .owner
            .as_ref()
            .filter(|owner| !same(entry.owner.as_deref(), owner));
        let group = self
            .group
            .as_ref()
            .filter(|group| !same(entry.group.as_deref(), group));
        if owner.is_none() && group.is_none() {
            return Ok(None);
        }
        let name = place
            .name()
            .map_or_else(|| place.to_string(), str::to_owned);
        let name = spell_out(&name).into_owned();
        let caller = &self.caller;
        if owner.is_some() && caller.user.is_some() && !caller.administrator {
            return Err(refused(self.words().administrator_only(&name)));
        }
        if let Some(group) = group
            && !caller.administrator
            && let Ok(gid) = group.id.parse::<u32>()
            && caller.in_group(gid) == Some(false)
        {
            return Err(refused(OwnerWords::not_your_group(&name, shown(group))));
        }
        change.owner = owner.cloned();
        change.group = group.cloned();
        Ok(Some(
            OwnerWords {
                owner: owner.map(shown),
                group: group.map(shown),
            }
            .change(),
        ))
    }
}

/// One planned change, before the steps are ordered.
struct Planned {
    place: Place,
    change: AttributeChange,
    /// Whether it goes after what is inside the folder: a folder whose
    /// owner loses list or enter, and every folder's times.
    after_contents: bool,
}

impl Planner<'_> {
    /// Permissions, attributes and times: every entry the edit reaches,
    /// with its new values computed from what it has, those that already
    /// match left out, ordered so a folder stays reachable while its
    /// contents change.
    fn attributes(&mut self, edit: &AttributeEdit) -> Result<(), OpsError> {
        if edit.is_empty() {
            return Err(OpsError::NoChange);
        }
        let mut planned: Vec<Planned> = Vec::new();
        let mut groups: BTreeMap<(bool, String), u64> = BTreeMap::new();
        let mut single_mode: Option<(u32, EntryKind)> = None;
        let mut created_note = false;
        let mut loses_access = 0u64;
        let mut everyone: Option<(Place, u64)> = None;
        let mut owner_groups: BTreeMap<(bool, String), u64> = BTreeMap::new();
        let request = self.request;
        // An owner change is checked at every place first, then its names
        // are looked up once.
        let owner_change = if edit.changes_owner() {
            for source in &request.sources {
                let provider = self.provider(source)?;
                OwnerChange::check_place(edit, source, &*provider)?;
            }
            let first = request
                .sources
                .first()
                .and_then(Place::name)
                .map(|name| spell_out(name).into_owned())
                .unwrap_or_default();
            Some(OwnerChange::resolve(edit, &first)?)
        } else {
            None
        };
        for source in &request.sources {
            if edit.recursion.goes_inside() {
                self.anchor(source, Removal::Permanent)?;
            }
            let name = Self::name_of(source)?;
            let provider = self.provider(source)?;
            let entry = provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            if entry.kind == EntryKind::Link {
                return Err(OpsError::LinkSelected(spell_out(&name).into_owned()));
            }
            let found = if edit.recursion.goes_inside() && entry.kind == EntryKind::Folder {
                self.walk(&*provider, source, entry)?
            } else {
                vec![Found {
                    place: source.clone(),
                    entry,
                    path: Vec::new(),
                }]
            };
            let capabilities = provider.capabilities(source);
            for item in found {
                let inside = item.depth() > 0;
                let kind = item.entry.kind;
                if kind == EntryKind::Link {
                    self.plan.facts.left_out.push((
                        item.place.clone(),
                        "a link: links have no permissions of their own".to_owned(),
                    ));
                    continue;
                }
                if inside && !edit.recursion.takes(kind) {
                    continue;
                }
                let mut change = AttributeChange::default();
                let mut after_contents = false;
                match (edit.mode, item.entry.attributes) {
                    (Some(mode), Some(Attributes::Unix(current))) => {
                        let new = mode.apply(current, kind, inside);
                        if new != current {
                            change.attributes = Some(Attributes::Unix(new));
                            change.expect = item.entry.attributes;
                            let words = change_words(current, new, kind);
                            *groups
                                .entry((kind == EntryKind::Folder, words))
                                .or_insert(0) += 1;
                            single_mode = Some((new, kind));
                            let owner_lost = (current & !new) & 0o500;
                            if kind == EntryKind::Folder && owner_lost != 0 {
                                after_contents = true;
                            }
                            let reads = if kind == EntryKind::Folder {
                                0o500
                            } else {
                                0o400
                            };
                            if current & reads != 0 && new & reads != current & reads {
                                loses_access += 1;
                            }
                            let opens = new & 0o002 != 0 && current & 0o002 == 0;
                            let sticky_folder = kind == EntryKind::Folder && new & 0o1000 != 0;
                            if opens && !sticky_folder {
                                let (_, n) = everyone.get_or_insert((item.place.clone(), 0));
                                *n += 1;
                            }
                            if kind != EntryKind::Folder
                                && new & 0o4000 != 0
                                && current & 0o4000 == 0
                            {
                                self.plan.facts.questions.push(Preflight::RunsAsOwner {
                                    place: item.place.clone(),
                                });
                            }
                        }
                    }
                    (Some(_), None) => {
                        self.plan.facts.left_out.push((
                            item.place.clone(),
                            "its permissions could not be read".to_owned(),
                        ));
                        continue;
                    }
                    _ => {}
                }
                if let (Some(windows), Some(Attributes::Windows(current))) =
                    (edit.windows, item.entry.attributes)
                {
                    let new = windows.apply(current);
                    if new != current {
                        change.attributes = Some(Attributes::Windows(new));
                        change.expect = item.entry.attributes;
                    }
                }
                if edit.modified.is_some() && edit.modified != item.entry.modified {
                    change.modified = edit.modified;
                }
                if edit.accessed.is_some() && edit.accessed != item.entry.accessed {
                    change.accessed = edit.accessed;
                }
                if edit.created.is_some() && edit.created != item.entry.created {
                    if capabilities.access.created_time.is_possible() {
                        change.created = edit.created;
                    } else {
                        created_note = true;
                    }
                }
                if let Some(owner_change) = &owner_change
                    && let Some(words) =
                        owner_change.apply(&item.place, &item.entry, &mut change)?
                {
                    *owner_groups
                        .entry((kind == EntryKind::Folder, words))
                        .or_insert(0) += 1;
                }
                if change.is_empty() {
                    continue;
                }
                if kind == EntryKind::Folder && change.attributes.is_none() {
                    // A folder's times are set after its contents.
                    after_contents = true;
                }
                self.plan.facts.count(&item.place, &item.entry);
                self.plan.expected.insert(item.place.clone(), item.entry);
                planned.push(Planned {
                    place: item.place,
                    change,
                    after_contents,
                });
            }
        }
        self.plan.steps.extend(order(planned));
        if let Some((place, items)) = everyone {
            self.plan
                .facts
                .questions
                .push(Preflight::EveryoneCanChange { place, items });
        }
        if loses_access > 0 {
            self.plan.facts.questions.push(Preflight::LosesAccess {
                items: loses_access,
            });
        }
        let mut rows = vec![edit.recursion.row()];
        let steps = self.plan.steps.len();
        match (single_mode, groups.len()) {
            (Some((mode, kind)), 1) if steps == 1 => rows.push(mode_line(mode, kind)),
            _ => rows.extend(groups.iter().map(|((folder, words), n)| {
                let kind = if *folder {
                    EntryKind::Folder
                } else {
                    EntryKind::File
                };
                group_row(*n, kind, words)
            })),
        }
        if edit.recursion.goes_inside() && matches!(edit.mode, Some(ModeChange::Bits { .. })) {
            rows.push("Files keep their run setting.".to_owned());
            rows.push("Folders stay enterable.".to_owned());
        }
        // "3 files: owner becomes ada", the owner's change in its own lines.
        rows.extend(owner_groups.iter().map(|((folder, words), n)| {
            let kind = if *folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            };
            group_row(*n, kind, words)
        }));
        if let Some(windows) = edit.windows {
            rows.extend(windows.rows());
        }
        if let Some(time) = edit.modified {
            rows.push(format!(
                "Modified time to {}",
                time_words(time, edit.offset)
            ));
        }
        if let Some(time) = edit.created {
            rows.push(format!("Created time to {}", time_words(time, edit.offset)));
        }
        if let Some(time) = edit.accessed {
            rows.push(format!(
                "Accessed time to {}",
                time_words(time, edit.offset)
            ));
        }
        if created_note {
            rows.push("Created time not set: this drive cannot keep one.".to_owned());
        }
        if self.plan.steps.is_empty() && self.plan.facts.left_out.is_empty() {
            let owners_only = edit.changes_owner() && edit.mode.is_none() && edit.windows.is_none();
            rows.push(if edit.times_only() {
                "Nothing to change: the times already match.".to_owned()
            } else if owners_only
                && edit.modified.is_none()
                && edit.created.is_none()
                && edit.accessed.is_none()
            {
                "Nothing to change: the owners already match.".to_owned()
            } else {
                "Nothing to change: permissions already match.".to_owned()
            });
        }
        // Bytes are not what this job is about.
        self.plan.facts.counted = false;
        self.plan.facts.attributes = Some(AttributeFacts {
            edit: edit.clone(),
            rows,
        });
        Ok(())
    }
}

/// The steps in order: the walk's order, folder before contents, except a
/// folder marked to go after its contents, which follows the last entry
/// inside it.
fn order(planned: Vec<Planned>) -> Vec<Step> {
    let mut steps = Vec::with_capacity(planned.len());
    let mut waiting: Vec<Planned> = Vec::new();
    let step = |planned: Planned| Step::SetAttributes {
        place: planned.place,
        change: planned.change,
    };
    for item in planned {
        while waiting
            .last()
            .is_some_and(|folder| !item.place.is_within(&folder.place))
        {
            if let Some(folder) = waiting.pop() {
                steps.push(step(folder));
            }
        }
        if item.after_contents {
            waiting.push(item);
        } else {
            steps.push(step(item));
        }
    }
    while let Some(folder) = waiting.pop() {
        steps.push(step(folder));
    }
    steps
}

impl Engine {
    /// Plans a permissions, attributes or times job: the request names the
    /// entries (and `SetAttributes` or `SetTimes` as its kind), and its
    /// edit (`JobRequest::attributes`, P2b) what changes and where; a
    /// request without one is refused as `NoChange`. A selected link is
    /// refused; a change that goes inside folders is refused at a drive
    /// root or a protected folder; the rows say the change in words, the
    /// recursion first.
    pub fn plan_attributes(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        let edit = request.attributes.as_ref().ok_or(OpsError::NoChange)?;
        if request.sources.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let quiet = Quiet;
        let control = JobControl::new();
        let mut planner = Planner::new(self, request, &control, &quiet);
        planner.plan.kind = match request.kind {
            JobKind::SetTimes => JobKind::SetTimes,
            _ if edit.times_only() => JobKind::SetTimes,
            _ => JobKind::SetAttributes,
        };
        planner.attributes(edit)?;
        Ok(planner.finish())
    }
}

#[cfg(test)]
mod owner_tests {
    //! The owner plan on a fake that keeps owners and groups (R30's
    //! commit 79): the names and numbers, the lines grouped, matches
    //! dropped, the refusals in words, the run and its undo.

    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    use diroxide_core::{Entry, JobId, JobKind, JobRequest, Place};
    use diroxide_vfs::access::Caller;
    use diroxide_vfs::{
        AttributeChange, Can, Capabilities, FileWriter, OpContext, Principal, Provider, Registry,
        VfsError, WriteMode,
    };

    use super::as_caller;
    use crate::access::{AttributeEdit, Recursion, UNDO_NEEDS_ADMINISTRATOR, undo_owner_line};
    use crate::engine::{Engine, JobControl};
    use crate::error::OpsError;
    use crate::fake::FakeProvider;
    use crate::testing::{Script, p};

    /// The fake, with an owner and a group for every entry, by number.
    #[derive(Clone, Debug)]
    struct Owned {
        fake: FakeProvider,
        owners: Arc<Mutex<HashMap<Place, (String, String)>>>,
    }

    impl Owned {
        fn new() -> Self {
            Self {
                fake: FakeProvider::new(),
                owners: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn file(&self, path: &str, owner: &str, group: &str) {
            let place = p(path);
            self.fake.add_file(&place, b"x");
            self.own(&place, owner, group);
        }

        fn own(&self, place: &Place, owner: &str, group: &str) {
            self.owners
                .lock()
                .unwrap()
                .insert(place.clone(), (owner.to_owned(), group.to_owned()));
        }

        fn owner_of(&self, path: &str) -> (String, String) {
            self.owners.lock().unwrap()[&p(path)].clone()
        }

        fn fill(&self, place: &Place, mut entry: Entry) -> Entry {
            if let Some((owner, group)) = self.owners.lock().unwrap().get(place) {
                entry.owner = Some(owner.clone());
                entry.group = Some(group.clone());
            }
            entry
        }
    }

    impl Provider for Owned {
        fn scheme(&self) -> &str {
            "file"
        }

        fn capabilities(&self, place: &Place) -> Capabilities {
            let mut capabilities = self.fake.capabilities(place);
            capabilities.access.owner = Can::WithAdministrator;
            capabilities.access.group = Can::Yes;
            capabilities
        }

        fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
            let entries = self.fake.list(place, ctx)?;
            Ok(entries
                .into_iter()
                .map(|entry| {
                    let at = place.join(&entry.name).expect("a child");
                    self.fill(&at, entry)
                })
                .collect())
        }

        fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
            Ok(self.fill(place, self.fake.stat(place, ctx)?))
        }

        fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
            self.fake.read(place, ctx)
        }

        fn write(
            &self,
            place: &Place,
            mode: WriteMode,
            ctx: &OpContext,
        ) -> Result<Box<dyn FileWriter>, VfsError> {
            self.fake.write(place, mode, ctx)
        }

        fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
            self.fake.create_dir(place, ctx)
        }

        fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
            self.fake.rename(from, to, ctx)
        }

        fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
            self.fake.remove(place, ctx)
        }

        fn set_attributes(
            &self,
            place: &Place,
            change: &AttributeChange,
            ctx: &OpContext,
        ) -> Result<AttributeChange, VfsError> {
            let rest = AttributeChange {
                owner: None,
                group: None,
                ..change.clone()
            };
            let mut inverse = if rest.is_empty() {
                AttributeChange::default()
            } else {
                self.fake.set_attributes(place, &rest, ctx)?
            };
            let mut owners = self.owners.lock().unwrap();
            let now = owners.entry(place.clone()).or_default();
            let principal = |id: &str| Principal {
                id: id.to_owned(),
                name: None,
            };
            if let Some(owner) = &change.owner {
                inverse.owner = Some(principal(&now.0));
                now.0.clone_from(&owner.id);
            }
            if let Some(group) = &change.group {
                inverse.group = Some(principal(&now.1));
                now.1.clone_from(&group.id);
            }
            Ok(inverse)
        }
    }

    fn engine(owned: &Owned) -> Engine {
        let mut registry = Registry::new();
        registry.set_local(Arc::new(owned.clone()));
        Engine::new(Arc::new(registry))
    }

    fn request(sources: &[&str], edit: AttributeEdit) -> JobRequest {
        JobRequest::new(
            JobId::new(79),
            JobKind::SetAttributes,
            sources.iter().map(|s| p(s)).collect(),
        )
        .changing(edit)
    }

    fn rows(plan: &crate::plan::Plan) -> Vec<String> {
        plan.facts.attributes.clone().unwrap().rows
    }

    /// A tree whose owners differ: 4242 is a number no test machine
    /// names, so the words are the number.
    fn tree() -> Owned {
        let owned = Owned::new();
        owned.fake.add_folder(&p("/t"));
        owned.own(&p("/t"), "100", "100");
        owned.file("/t/a.txt", "4242", "100");
        owned.file("/t/b.txt", "100", "100");
        owned.fake.add_folder(&p("/t/sub"));
        owned.own(&p("/t/sub"), "100", "100");
        owned.file("/t/sub/c.txt", "100", "100");
        owned
    }

    #[test]
    fn an_administrators_owner_change_groups_its_lines_and_drops_matches() {
        let owned = tree();
        let engine = engine(&owned);
        let edit = AttributeEdit::new(Recursion::FilesAndFolders).with_owner("4242");
        let plan = as_caller(Caller::administrator(), || {
            engine.plan_attributes(&request(&["/t"], edit)).unwrap()
        });
        assert_eq!(
            rows(&plan),
            [
                "Apply to: files and folders inside",
                "2 files: owner becomes 4242",
                "2 folders: owner becomes 4242",
            ]
        );
        assert_eq!(plan.steps.len(), 4, "a.txt already matches");
        let report = engine.run(&plan, &JobControl::new(), &Script::default());
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(owned.owner_of("/t/b.txt").0, "4242");
        assert_eq!(owned.owner_of("/t/sub").0, "4242");
        // The undo puts the old owners back, and says it needs an
        // administrator when the caller is not one.
        assert_eq!(
            undo_owner_line(&report.undo.attributes, &Caller::user(1000, vec![1000])),
            Some(UNDO_NEEDS_ADMINISTRATOR)
        );
        assert_eq!(
            undo_owner_line(&report.undo.attributes, &Caller::administrator()),
            None
        );
        let undo = engine.plan_undo(&report.undo, JobId::new(80)).unwrap();
        engine.run(&undo, &JobControl::new(), &Script::default());
        assert_eq!(owned.owner_of("/t/b.txt").0, "100");
        assert_eq!(owned.owner_of("/t/a.txt").0, "4242", "never changed");
    }

    #[test]
    fn owner_and_group_together_read_as_one_line() {
        let owned = tree();
        let engine = engine(&owned);
        let edit = AttributeEdit::new(Recursion::ThisItem)
            .with_owner("4242")
            .with_group("4343");
        let plan = as_caller(Caller::administrator(), || {
            engine
                .plan_attributes(&request(&["/t/b.txt"], edit))
                .unwrap()
        });
        assert_eq!(
            rows(&plan)[1],
            "1 file: owner becomes 4242, group becomes 4343"
        );
        let same = AttributeEdit::new(Recursion::ThisItem).with_owner("4242");
        let plan = as_caller(Caller::administrator(), || {
            engine
                .plan_attributes(&request(&["/t/a.txt"], same))
                .unwrap()
        });
        assert!(plan.steps.is_empty());
        assert_eq!(
            rows(&plan)[1],
            "Nothing to change: the owners already match."
        );
    }

    fn refusal(result: Result<crate::plan::Plan, OpsError>) -> String {
        match result {
            Err(OpsError::OwnerRefused(sentence)) => sentence,
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn what_only_an_administrator_could_do_is_refused_in_words() {
        let owned = tree();
        let engine = engine(&owned);
        let me = Caller::user(100, vec![100, 4344]);
        let owner = AttributeEdit::new(Recursion::ThisItem).with_owner("4242");
        let sentence = as_caller(me.clone(), || {
            refusal(engine.plan_attributes(&request(&["/t/b.txt"], owner)))
        });
        assert_eq!(
            sentence,
            "Could not change the owner of b.txt: only an administrator can. Start diro as administrator, then try again."
        );
        let theirs = AttributeEdit::new(Recursion::ThisItem).with_group("4345");
        let sentence = as_caller(me.clone(), || {
            refusal(engine.plan_attributes(&request(&["/t/b.txt"], theirs)))
        });
        assert_eq!(
            sentence,
            "Could not change the group of b.txt: you are not in group 4345. Choose one of your groups."
        );
        // One of the caller's own groups is planned.
        let mine = AttributeEdit::new(Recursion::ThisItem).with_group("4344");
        let plan = as_caller(me, || {
            engine
                .plan_attributes(&request(&["/t/b.txt"], mine))
                .unwrap()
        });
        assert_eq!(rows(&plan)[1], "1 file: group becomes 4344");
        // A caller the system cannot name (macOS) lets the system decide.
        let unknown = AttributeEdit::new(Recursion::ThisItem).with_owner("4242");
        let plan = as_caller(Caller::default(), || {
            engine
                .plan_attributes(&request(&["/t/b.txt"], unknown))
                .unwrap()
        });
        assert_eq!(plan.steps.len(), 1);
    }

    #[test]
    fn unknown_names_protected_places_and_places_without_owners_are_refused() {
        let owned = tree();
        let engine = engine(&owned);
        let edit = AttributeEdit::new(Recursion::ThisItem).with_owner("no-such-user-here");
        let sentence = as_caller(Caller::administrator(), || {
            refusal(engine.plan_attributes(&request(&["/t/b.txt"], edit)))
        });
        assert_eq!(
            sentence,
            "Could not change the owner of b.txt: there is no user named no-such-user-here here. Type a user name or a number."
        );
        let top = AttributeEdit::new(Recursion::ThisItem).with_owner("0");
        let sentence = as_caller(Caller::administrator(), || {
            refusal(engine.plan_attributes(&request(&["/"], top)))
        });
        assert!(
            sentence.starts_with("Could not change the owner of ")
                && sentence
                    .ends_with(": the top of a drive is protected. Use the system's own tools."),
            "{sentence}"
        );
        // The plain fake keeps no owners: on Windows that is the local
        // provider, which hands off to the system's Security dialog.
        let plain = FakeProvider::new();
        plain.add_file(&p("/w/a.txt"), b"x");
        let engine = crate::testing::engine(&plain);
        let group = AttributeEdit::new(Recursion::ThisItem).with_group("27");
        let sentence = as_caller(Caller::administrator(), || {
            refusal(engine.plan_attributes(&request(&["/w/a.txt"], group)))
        });
        let expected = if cfg!(windows) {
            "Could not change the group of a.txt: Windows does it in its Security dialog. Open the Security dialog instead."
        } else {
            "Could not change the group of a.txt: this place keeps no groups."
        };
        assert_eq!(sentence, expected);
    }

    /// A folder for a test on the real disk: under the build's own folder,
    /// or where `DIRO_OWNER_TEST_DIR` says (the container's run as a user
    /// made for the test, which cannot write the build's folder).
    #[cfg(unix)]
    fn scratch() -> tempfile::TempDir {
        let base = std::env::var_os("DIRO_OWNER_TEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let exe = std::env::current_exe().unwrap();
                exe.parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("owner-unit-tmp")
            });
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("owner-")
            .tempdir_in(&base)
            .unwrap()
    }

    /// The owner plan on the real disk, run and undone: a group of the
    /// caller's own; as root, an owner too; as anyone else, the owner
    /// refused in words. Nothing outside its own folder changes.
    #[cfg(unix)]
    #[test]
    fn the_local_disk_changes_a_group_and_as_root_an_owner_and_undoes_them() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch();
        let path = dir.path().join("mine.txt");
        std::fs::write(&path, b"x").unwrap();
        let mut registry = Registry::new();
        registry.set_local(Arc::new(diroxide_vfs::LocalProvider::new()));
        let engine = Engine::new(Arc::new(registry));
        let place = Place::parse(path.to_str().unwrap()).unwrap();
        let request = |edit: AttributeEdit| {
            JobRequest::new(JobId::new(81), JobKind::SetAttributes, vec![place.clone()])
                .changing(edit)
        };
        let caller = Caller::current();
        let before = std::fs::metadata(&path).unwrap();
        let other_group = caller
            .groups
            .iter()
            .flatten()
            .copied()
            .find(|gid| *gid != before.gid());
        if let Some(gid) = other_group {
            let edit = AttributeEdit::new(Recursion::ThisItem).with_group(gid.to_string());
            let plan = engine.plan_attributes(&request(edit)).unwrap();
            let report = engine.run(&plan, &JobControl::new(), &Script::default());
            assert!(report.failed.is_empty(), "{:?}", report.failed);
            assert_eq!(std::fs::metadata(&path).unwrap().gid(), gid);
            let undo = engine.plan_undo(&report.undo, JobId::new(82)).unwrap();
            engine.run(&undo, &JobControl::new(), &Script::default());
            assert_eq!(std::fs::metadata(&path).unwrap().gid(), before.gid());
        }
        let owner = AttributeEdit::new(Recursion::ThisItem).with_owner("4242");
        if caller.administrator {
            let plan = engine.plan_attributes(&request(owner)).unwrap();
            let report = engine.run(&plan, &JobControl::new(), &Script::default());
            assert!(report.failed.is_empty(), "{:?}", report.failed);
            assert_eq!(std::fs::metadata(&path).unwrap().uid(), 4242);
            let undo = engine.plan_undo(&report.undo, JobId::new(83)).unwrap();
            engine.run(&undo, &JobControl::new(), &Script::default());
            assert_eq!(std::fs::metadata(&path).unwrap().uid(), before.uid());
        } else if caller.user.is_some() {
            let sentence = refusal(engine.plan_attributes(&request(owner)));
            assert_eq!(
                sentence,
                "Could not change the owner of mine.txt: only an administrator can. Start diro as administrator, then try again."
            );
        }
    }
}
