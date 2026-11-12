//! The selection helpers: which copy to keep in each group, the rest
//! marked. A helper never marks a whole group; a group it cannot decide
//! is left alone and said.

use diroxide_core::Place;
use diroxide_core::format::human_size;

use super::{DupGroup, DupResult, grouped_count};

/// Which copy of each group to keep. Every rule keeps one copy and
/// settles a tie the same way: the shortest full path, then the first
/// found. [`KeepRule::InFolder`] keeps every copy in its folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KeepRule {
    /// The one changed first: the default (plan 5.5, the owner's answer
    /// 54). A group with a time not known is left alone and said.
    #[default]
    Oldest,
    /// The one changed last. A group with a time not known is left alone
    /// and said.
    Newest,
    /// The one with the shortest full path; a tie goes to the first
    /// found.
    ShortestPath,
    /// The one whose name comes first, case ignored; a tie goes to the
    /// shortest path, then the first found.
    FirstByName,
    /// The one the search found first.
    FirstFound,
    /// The copies directly in this folder. A group with no copy there,
    /// or with every copy there, is left alone and said.
    InFolder(Place),
}

impl KeepRule {
    /// The rule in words, after "Kept": "the newest", "the copies in
    /// D:\a".
    pub fn words(&self) -> String {
        match self {
            KeepRule::Newest => "the newest".to_owned(),
            KeepRule::Oldest => "the oldest".to_owned(),
            KeepRule::FirstFound => "the first found".to_owned(),
            KeepRule::ShortestPath => "the shortest path".to_owned(),
            KeepRule::FirstByName => "the first by name".to_owned(),
            KeepRule::InFolder(folder) => format!("the copies in {folder}"),
        }
    }
}

/// What a helper chose in one group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupChoice {
    /// The group, by its index in [`DupResult::groups`].
    pub group: usize,
    /// The members kept, by index in the group's files.
    pub keep: Vec<usize>,
    /// The members marked, by index; never every member.
    pub mark: Vec<usize>,
    /// Why nothing is marked, when the group is left alone: "no copy in
    /// D:\a", "a time is not known".
    pub left_alone: Option<String>,
}

/// What a helper chose in every group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The rule.
    pub rule: KeepRule,
    /// One choice per group, in the result's order.
    pub choices: Vec<GroupChoice>,
    /// Every marked place, group by group.
    pub marked: Vec<Place>,
    /// The bytes the marked files free.
    pub bytes: u64,
}

impl Selection {
    /// The groups with something marked.
    pub fn groups_marked(&self) -> usize {
        self.choices.iter().filter(|c| !c.mark.is_empty()).count()
    }

    /// The line said when the helper runs, the count first: "Marked 82
    /// files, 1.4 GB. Kept the newest in 41 groups.", or "Marked nothing.
    /// 3 groups left alone."
    pub fn summary(&self) -> String {
        let alone = self.choices.len() - self.groups_marked();
        if self.marked.is_empty() {
            return format!(
                "Marked nothing. {} left alone.",
                grouped_count(alone as u64, "group", "groups")
            );
        }
        let mut text = format!(
            "Marked {}, {}. Kept {} in {}.",
            grouped_count(self.marked.len() as u64, "file", "files"),
            human_size(self.bytes),
            self.rule.words(),
            grouped_count(self.groups_marked() as u64, "group", "groups")
        );
        if alone > 0 {
            text.push_str(&format!(
                " {} left alone.",
                grouped_count(alone as u64, "group", "groups")
            ));
        }
        text
    }

    /// One line per group left alone, the group first: "Group 3 left
    /// alone: no copy in D:\a".
    pub fn left_alone_lines(&self) -> Vec<String> {
        self.choices
            .iter()
            .filter_map(|choice| {
                choice
                    .left_alone
                    .as_ref()
                    .map(|why| format!("Group {} left alone: {why}", choice.group + 1))
            })
            .collect()
    }
}

/// Marks every copy but the ones `rule` keeps, in every group of
/// `result`. The names of a kept file's hard links stay unmarked, since
/// trashing them frees nothing.
pub fn select(result: &DupResult, rule: &KeepRule) -> Selection {
    let mut choices = Vec::new();
    let mut marked = Vec::new();
    let mut bytes = 0u64;
    for (index, group) in result.groups.iter().enumerate() {
        let choice = choose(index, group, rule);
        let mut roots_marked = Vec::new();
        for &at in &choice.mark {
            marked.push(group.files[at].place.clone());
            if group.files[at].same_file_as.is_none() {
                roots_marked.push(at);
            }
        }
        bytes = bytes.saturating_add(group.size.saturating_mul(roots_marked.len() as u64));
        choices.push(choice);
    }
    Selection {
        rule: rule.clone(),
        choices,
        marked,
        bytes,
    }
}

/// The member a name stands for: itself, or the file it is a link of.
fn root(group: &DupGroup, at: usize) -> usize {
    group.files[at].same_file_as.unwrap_or(at)
}

fn alone(index: usize, why: String) -> GroupChoice {
    GroupChoice {
        group: index,
        keep: Vec::new(),
        mark: Vec::new(),
        left_alone: Some(why),
    }
}

fn choose(index: usize, group: &DupGroup, rule: &KeepRule) -> GroupChoice {
    let distinct: Vec<usize> = (0..group.files.len())
        .filter(|&at| group.files[at].same_file_as.is_none())
        .collect();
    let path_length = |at: usize| group.files[at].place.to_string().chars().count();
    // The tie rule of every helper: the shortest path, then the first
    // found.
    let settle = |candidates: &[usize]| {
        candidates
            .iter()
            .copied()
            .min_by_key(|&at| (path_length(at), group.files[at].found))
    };
    let first_found = |candidates: &[usize]| {
        candidates
            .iter()
            .copied()
            .min_by_key(|&at| group.files[at].found)
    };
    let keep: Vec<usize> = match rule {
        KeepRule::Newest | KeepRule::Oldest => {
            let times: Option<Vec<_>> = distinct
                .iter()
                .map(|&at| group.files[at].modified.map(|t| (at, t)))
                .collect();
            let Some(times) = times else {
                return alone(index, "a time is not known".to_owned());
            };
            let best = if *rule == KeepRule::Newest {
                times.iter().map(|&(_, t)| t).max()
            } else {
                times.iter().map(|&(_, t)| t).min()
            };
            let ties: Vec<usize> = times
                .iter()
                .filter(|&&(_, t)| Some(t) == best)
                .map(|&(at, _)| at)
                .collect();
            settle(&ties).into_iter().collect()
        }
        KeepRule::FirstFound => first_found(&distinct).into_iter().collect(),
        KeepRule::ShortestPath => settle(&distinct).into_iter().collect(),
        KeepRule::FirstByName => {
            let name = |at: usize| {
                group.files[at]
                    .place
                    .name()
                    .unwrap_or_default()
                    .to_lowercase()
            };
            let first = distinct.iter().map(|&at| name(at)).min();
            let ties: Vec<usize> = distinct
                .iter()
                .copied()
                .filter(|&at| Some(name(at)) == first)
                .collect();
            settle(&ties).into_iter().collect()
        }
        KeepRule::InFolder(folder) => {
            let here: Vec<usize> = distinct
                .iter()
                .copied()
                .filter(|&at| group.files[at].place.parent().as_ref() == Some(folder))
                .collect();
            if here.is_empty() {
                return alone(index, format!("no copy in {folder}"));
            }
            if here.len() == distinct.len() {
                return alone(index, format!("every copy is in {folder}"));
            }
            here
        }
    };
    if keep.is_empty() {
        return alone(index, "nothing to keep".to_owned());
    }
    let mark: Vec<usize> = (0..group.files.len())
        .filter(|&at| !keep.contains(&root(group, at)))
        .collect();
    if mark.is_empty() {
        return alone(index, "every copy is kept".to_owned());
    }
    GroupChoice {
        group: index,
        keep,
        mark,
        left_alone: None,
    }
}
