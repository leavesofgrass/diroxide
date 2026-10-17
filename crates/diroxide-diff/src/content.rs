//! Comparing two files' contents as text: each decoded the way the viewer
//! decodes it, its facts read, and the texts compared line by line.

use diroxide_vfs::CancelToken;

use crate::error::DiffError;
use crate::explain::{Reason, explain};
use crate::facts::{FactDifference, FileFacts, fact_differences};
use crate::options::CompareOptions;
use crate::text::{TextComparison, compare_text};
use crate::words::{ChangeWindow, WordDiff, change_window, word_diff};

/// Two files' contents compared as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentComparison {
    before: FileFacts,
    after: FileFacts,
    identical: bool,
    text: TextComparison,
}

/// Compares two files' contents as text under `options`, honoring
/// `cancel` and the deadline.
pub fn compare_contents(
    before: &[u8],
    after: &[u8],
    options: &CompareOptions,
    cancel: &CancelToken,
) -> Result<ContentComparison, DiffError> {
    let (before_facts, before_text) = FileFacts::read(before);
    let (after_facts, after_text) = FileFacts::read(after);
    let text = compare_text(&before_text, &after_text, options, cancel)?;
    Ok(ContentComparison {
        before: before_facts,
        after: after_facts,
        identical: before == after,
        text,
    })
}

impl ContentComparison {
    /// The before file's facts.
    pub fn before_facts(&self) -> &FileFacts {
        &self.before
    }

    /// The after file's facts.
    pub fn after_facts(&self) -> &FileFacts {
        &self.after
    }

    /// Whether the two files hold the same bytes.
    pub fn identical(&self) -> bool {
        self.identical
    }

    /// The line-by-line comparison of the texts.
    pub fn text(&self) -> &TextComparison {
        &self.text
    }

    /// The facts about the files as wholes that differ, in reading order.
    pub fn fact_differences(&self) -> Vec<FactDifference> {
        fact_differences(&self.before, &self.after, self.text.ignore().line_endings)
    }

    /// The same comparison read the other way (Swap).
    pub fn swapped(&self) -> ContentComparison {
        ContentComparison {
            before: self.after.clone(),
            after: self.before.clone(),
            identical: self.identical,
            text: self.text.swapped(),
        }
    }

    /// Why a changed line pair sounds the same, for a change of one line
    /// on each side; `None` for any other change, or when the lines sound
    /// different.
    pub fn reason(&self, change: usize) -> Option<Reason> {
        let change = self.text.changes().get(change)?;
        if change.before.len() != 1 || change.after.len() != 1 {
            return None;
        }
        self.pair_reason(change.before.start, change.after.start)
    }

    /// Why the before line `before` and the after line `after` sound the
    /// same, if they do.
    pub fn pair_reason(&self, before: usize, after: usize) -> Option<Reason> {
        let text = &self.text;
        explain(
            text.before_line(before)?,
            text.before_ending(before)?,
            text.after_line(after)?,
            text.after_ending(after)?,
        )
    }

    /// The word pass over a before line and an after line, within the
    /// compare's deadline from now.
    pub fn pair_words(
        &self,
        before: usize,
        after: usize,
        options: &CompareOptions,
        cancel: &CancelToken,
    ) -> Result<Option<WordDiff>, DiffError> {
        let (Some(b), Some(a)) = (self.text.before_line(before), self.text.after_line(after))
        else {
            return Ok(None);
        };
        let deadline = std::time::Instant::now().checked_add(options.deadline);
        word_diff(b, a, deadline, cancel).map(Some)
    }

    /// The change window of a before line and an after line.
    pub fn pair_window(
        &self,
        before: usize,
        after: usize,
        options: &CompareOptions,
        cancel: &CancelToken,
    ) -> Result<Option<ChangeWindow>, DiffError> {
        let Some(words) = self.pair_words(before, after, options, cancel)? else {
            return Ok(None);
        };
        let (Some(b), Some(a)) = (self.text.before_line(before), self.text.after_line(after))
        else {
            return Ok(None);
        };
        Ok(change_window(b, a, &words))
    }
}
