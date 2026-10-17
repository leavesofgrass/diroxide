//! Link plans (research/alpha2-plan.md commit 26): links made relative or
//! absolute, and changing where a link points.

use diroxide_core::{ConflictPolicy, EntryKind, JobKind, JobRequest, Place};
use diroxide_vfs::{LinkKind, LinkRequest, OpContext};

use super::Planner;
use crate::engine::{Engine, JobControl, Quiet};
use crate::error::OpsError;
use crate::links::{LinkForm, relative_target};
use crate::plan::{Plan, Step};

impl Planner<'_> {
    /// The target text of a new link at `place` to `source`: the full
    /// path, or the way from the link's folder. A way that does not exist
    /// (another drive), or a kind that needs the full path, falls back to
    /// it and says so in the plan's note.
    pub(super) fn link_target(
        &mut self,
        place: &Place,
        source: &Place,
        kind: LinkKind,
        form: LinkForm,
    ) -> String {
        match (form, kind) {
            (LinkForm::Absolute, _) => source.to_string(),
            (LinkForm::Relative, LinkKind::Symbolic) => match relative_target(place, source) {
                Some(relative) => relative,
                None => {
                    self.plan.note =
                        Some("Absolute link: the target is on another drive.".to_owned());
                    source.to_string()
                }
            },
            (LinkForm::Relative, other) => {
                let what = if other == LinkKind::Junction {
                    "A junction"
                } else {
                    "A hard link"
                };
                self.plan.note = Some(format!("{what} points by the full path."));
                source.to_string()
            }
        }
    }
}

impl Engine {
    /// Plans links as the request's [`LinkEdit`](diroxide_core::LinkEdit)
    /// says (`JobRequest::link`; symbolic and absolute without one): one in
    /// the destination for each source, absolute or relative to the link's
    /// folder. An edit that retargets plans [`Engine::plan_relink`]
    /// instead. Refused before anything is listed where the destination
    /// holds no links.
    pub fn plan_links_as(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        let edit = request.link_edit();
        if edit.retarget {
            return self.plan_relink(request);
        }
        if request.destination.is_none() {
            return Err(OpsError::NoDestination);
        }
        if request.sources.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let quiet = Quiet;
        let control = JobControl::new();
        let mut planner = Planner::new(self, request, &control, &quiet);
        planner.plan.kind = JobKind::Link;
        planner.link(edit.kind, edit.form)?;
        Ok(planner.finish())
    }

    /// Plans changing where the request's one source link points, to its
    /// destination, in the form of its [`LinkEdit`](diroxide_core::LinkEdit)
    /// (absolute without one), as job `request.id`: one step that makes the
    /// new link beside the old and renames it over the old one in one step
    /// where the place can (`replace_rename`), or else sends the old one to
    /// the trash and gives the new one its name; Undo brings the old link
    /// back either way. Refused where the place holds no links, can neither
    /// replace in one step nor keep the old one in a trash, or the source
    /// is not a link.
    pub fn plan_relink(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        let [link] = request.sources.as_slice() else {
            return Err(OpsError::NothingSelected);
        };
        let target = request
            .destination
            .as_ref()
            .ok_or(OpsError::NoDestination)?;
        let (job, form) = (request.id, request.link_edit().form);
        let ctx = OpContext::new();
        let provider = self.registry().resolve(link)?;
        let entry = provider.stat(link, &ctx)?;
        let name = link.name().ok_or(OpsError::NothingSelected)?;
        if entry.kind != EntryKind::Link {
            return Err(OpsError::NameRefused(format!(
                "Not a link: {}",
                diroxide_core::names::spell_out(name)
            )));
        }
        let folder = link.parent().ok_or(OpsError::NothingSelected)?;
        let capabilities = provider.capabilities(&folder);
        if !capabilities.can_link() {
            return Err(OpsError::CannotLink(
                capabilities.links_in_words().to_owned(),
            ));
        }
        if !capabilities.trash && !capabilities.replace_rename {
            return Err(OpsError::CannotLink(
                "Cannot change a link here: no trash keeps the old one".to_owned(),
            ));
        }
        let mut note = None;
        let target_is_folder = match self
            .registry()
            .resolve(target)
            .and_then(|p| p.stat(target, &ctx))
        {
            Ok(found) => found.kind == EntryKind::Folder,
            Err(_) => {
                note = Some("The new target is not there yet.".to_owned());
                false
            }
        };
        let text = match form {
            LinkForm::Absolute => target.to_string(),
            LinkForm::Relative => relative_target(link, target).unwrap_or_else(|| {
                note = Some("Absolute link: the target is on another drive.".to_owned());
                target.to_string()
            }),
        };
        let mut plan = Plan::new(
            job,
            JobKind::Link,
            vec![Step::Relink {
                place: link.clone(),
                request: LinkRequest {
                    kind: LinkKind::Symbolic,
                    target: text,
                    target_is_folder,
                },
            }],
            ConflictPolicy::Skip,
        );
        plan.note = note;
        plan.facts.items = 1;
        plan.facts.links = 1;
        plan.facts.destination = Some(folder);
        plan.expected.insert(link.clone(), entry);
        Ok(plan)
    }
}
