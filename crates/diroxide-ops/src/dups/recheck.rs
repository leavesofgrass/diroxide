//! The check before the trash (R23, "A cleanup that removes the
//! original"): every keeper and every marked file read again. A group
//! whose kept copy is gone or changed is refused whole, so nothing of it
//! goes; a marked file that changed is no longer a copy and is dropped.

use std::sync::Arc;

use diroxide_core::Place;
use diroxide_core::names::display_name;
use diroxide_vfs::{OpContext, Provider, VfsError};

use super::find::{Ticker, full, same_bytes};
use super::{
    DupEnv, DupError, DupGroup, DupOptions, DupResult, DupStage, Selection, grouped_count,
};
use crate::engine::JobControl;

/// One thing the check held back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The group, by its index in [`DupResult::groups`].
    pub group: usize,
    /// The file that failed the check: a keeper, for a group refused
    /// whole; a marked file, for one dropped.
    pub place: Place,
    /// Whether the whole group was refused.
    pub whole_group: bool,
    /// Why: "is gone", "changed".
    pub why: String,
}

impl Refusal {
    /// The line, the verdict first: "Group 3 refused: the kept copy,
    /// report.pdf, changed", or "Not trashed: notes.txt changed, group 3".
    pub fn line(&self) -> String {
        let name = self
            .place
            .name()
            .map(|n| display_name(n).into_owned())
            .unwrap_or_else(|| self.place.to_string());
        if self.whole_group {
            format!(
                "Group {} refused: the kept copy, {name}, {}",
                self.group + 1,
                self.why
            )
        } else {
            format!("Not trashed: {name} {}, group {}", self.why, self.group + 1)
        }
    }
}

/// What the check let through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rechecked {
    /// The marked places still copies of an intact keeper: what the app
    /// sends to the trash.
    pub safe: Vec<Place>,
    /// What was held back, each with its reason.
    pub refused: Vec<Refusal>,
}

impl Rechecked {
    /// The line, the count first: "Checked again: 82 files can go to the
    /// trash", with "2 held back" after when any were.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "Checked again: {} can go to the trash",
            grouped_count(self.safe.len() as u64, "file", "files")
        );
        if !self.refused.is_empty() {
            text.push_str(&format!(", {} held back", self.refused.len()));
        }
        text
    }
}

/// Reads every keeper and every marked file of `selection` again,
/// against the digest `result` holds. Cancel stops it with
/// [`DupError::Cancelled`] and lets nothing through.
pub fn recheck_selection(
    result: &DupResult,
    selection: &Selection,
    control: &JobControl,
    env: &DupEnv<'_>,
) -> Result<Rechecked, DupError> {
    let ctx = OpContext::new().with_cancel(control.cancel.clone());
    let mut ticker = Ticker::new(env, DupOptions::default().progress_interval);
    let work: Vec<(&DupGroup, usize, Vec<usize>)> = selection
        .choices
        .iter()
        .filter(|c| !c.mark.is_empty())
        .filter_map(|c| {
            let group = result.groups.get(c.group)?;
            let reads = c.keep.iter().chain(&c.mark).copied().collect();
            Some((group, c.group, reads))
        })
        .collect();
    let files = work.iter().map(|(_, _, r)| r.len() as u64).sum();
    let bytes = work
        .iter()
        .map(|(g, _, r)| g.size.saturating_mul(r.len() as u64))
        .sum();
    ticker.begin(DupStage::Rechecking, files, bytes);
    let mut checked = Rechecked {
        safe: Vec::new(),
        refused: Vec::new(),
    };
    for choice in selection.choices.iter().filter(|c| !c.mark.is_empty()) {
        let Some(group) = result.groups.get(choice.group) else {
            continue;
        };
        let mut intact = true;
        let mut keeper: Option<(Arc<dyn Provider>, Place)> = None;
        for &at in &choice.keep {
            let place = &group.files[at].place;
            ticker.checkpoint(control)?;
            match same(env, group, place, &ctx, &mut ticker, control, None)? {
                Ok(provider) => {
                    if keeper.is_none() {
                        keeper = Some((provider, place.clone()));
                    }
                }
                Err(why) => {
                    checked.refused.push(Refusal {
                        group: choice.group,
                        place: place.clone(),
                        whole_group: true,
                        why,
                    });
                    intact = false;
                    break;
                }
            }
        }
        let Some(keeper) = keeper.filter(|_| intact) else {
            continue;
        };
        for &at in &choice.mark {
            let place = &group.files[at].place;
            ticker.checkpoint(control)?;
            let against = group.bytes_compared.then_some(&keeper);
            match same(env, group, place, &ctx, &mut ticker, control, against)? {
                Ok(_) => checked.safe.push(place.clone()),
                Err(why) => checked.refused.push(Refusal {
                    group: choice.group,
                    place: place.clone(),
                    whole_group: false,
                    why,
                }),
            }
        }
    }
    Ok(checked)
}

/// Whether one file still has the group's size and digest (and, with
/// `against`, the keeper's bytes): its provider, or why not in words.
fn same(
    env: &DupEnv<'_>,
    group: &DupGroup,
    place: &Place,
    ctx: &OpContext,
    ticker: &mut Ticker<'_>,
    control: &JobControl,
    against: Option<&(Arc<dyn Provider>, Place)>,
) -> Result<Result<Arc<dyn Provider>, String>, DupError> {
    let cancelled = |error: &VfsError| matches!(error, VfsError::Cancelled);
    let provider = match env.registry.resolve(place) {
        Ok(provider) => provider,
        Err(error) => return Ok(Err(format!("cannot be read: {error}"))),
    };
    ticker.progress.current = place.name().map(str::to_owned);
    let entry = match provider.stat(place, ctx) {
        Ok(entry) => entry,
        Err(VfsError::NotFound) => return Ok(Err("is gone".to_owned())),
        Err(error) if cancelled(&error) => return Err(DupError::Cancelled),
        Err(error) => return Ok(Err(format!("cannot be read: {error}"))),
    };
    if entry.size != Some(group.size) {
        return Ok(Err("changed".to_owned()));
    }
    let base = ticker.progress.bytes_done;
    if group.size > 0 {
        let digest = full(
            provider.as_ref(),
            place,
            group.size,
            ctx,
            env.hasher,
            &mut |done| {
                ticker.hold(control);
                ticker.progress.bytes_done = base + done;
                ticker.tick();
            },
        );
        match digest {
            Ok(digest) if digest == group.digest => {}
            Ok(_) | Err(VfsError::Changed) => return Ok(Err("changed".to_owned())),
            Err(VfsError::NotFound) => return Ok(Err("is gone".to_owned())),
            Err(error) if cancelled(&error) => return Err(DupError::Cancelled),
            Err(error) => return Ok(Err(format!("cannot be read: {error}"))),
        }
        if let Some((keeper_provider, keeper)) = against {
            match same_bytes(
                keeper_provider.as_ref(),
                keeper,
                provider.as_ref(),
                place,
                ctx,
                &mut |_| {},
            ) {
                Ok(true) => {}
                Ok(false) => return Ok(Err("changed".to_owned())),
                Err(error) if cancelled(&error) => return Err(DupError::Cancelled),
                Err(error) => return Ok(Err(format!("cannot be read: {error}"))),
            }
        }
    }
    ticker.progress.bytes_done = base + group.size;
    ticker.progress.files_done += 1;
    ticker.tick();
    Ok(Ok(provider))
}
