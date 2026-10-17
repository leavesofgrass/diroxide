//! The engine: the one module that names `similar` (R18 section 4). It
//! lines up two sequences of interned ids and returns stretches that are
//! the same or different. `imara-diff`, the documented swap if the bench
//! gate (plan principle 8) finds line compares too slow, would replace
//! this file and nothing else.
//!
//! Cancellation: a guard around the hook checks the token at every step
//! the algorithm reports, and the deadline bounds the stretches between
//! steps, so a cancel is honored within the deadline at worst.

use std::convert::Infallible;
use std::ops::Range;
use std::time::Instant;

use diroxide_vfs::CancelToken;
use similar::algorithms::{Capture, Compact, DiffHook, Replace, diff_deadline};
use similar::{Algorithm, DiffOp};

use crate::options::LineAlgorithm;

/// One stretch of the alignment: the same in both sequences, or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stretch {
    /// Whether the two ranges hold the same ids, pair by pair.
    pub(crate) same: bool,
    /// The range in the before sequence.
    pub(crate) before: Range<usize>,
    /// The range in the after sequence.
    pub(crate) after: Range<usize>,
}

/// The run was cancelled through its token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stopped;

/// Checks the token at every step, then hands the step on.
struct Guard<'c, D> {
    inner: D,
    cancel: &'c CancelToken,
}

impl<D> Guard<'_, D> {
    fn check(&self) -> Result<(), Stopped> {
        if self.cancel.is_cancelled() {
            Err(Stopped)
        } else {
            Ok(())
        }
    }
}

fn never(never: Infallible) -> Stopped {
    match never {}
}

impl<D: DiffHook<Error = Infallible>> DiffHook for Guard<'_, D> {
    type Error = Stopped;

    fn equal(&mut self, old_index: usize, new_index: usize, len: usize) -> Result<(), Stopped> {
        self.check()?;
        self.inner.equal(old_index, new_index, len).map_err(never)
    }

    fn delete(
        &mut self,
        old_index: usize,
        old_len: usize,
        new_index: usize,
    ) -> Result<(), Stopped> {
        self.check()?;
        self.inner
            .delete(old_index, old_len, new_index)
            .map_err(never)
    }

    fn insert(
        &mut self,
        old_index: usize,
        new_index: usize,
        new_len: usize,
    ) -> Result<(), Stopped> {
        self.check()?;
        self.inner
            .insert(old_index, new_index, new_len)
            .map_err(never)
    }

    fn replace(
        &mut self,
        old_index: usize,
        old_len: usize,
        new_index: usize,
        new_len: usize,
    ) -> Result<(), Stopped> {
        self.check()?;
        self.inner
            .replace(old_index, old_len, new_index, new_len)
            .map_err(never)
    }

    fn finish(&mut self) -> Result<(), Stopped> {
        self.check()?;
        self.inner.finish().map_err(never)
    }
}

const fn algorithm(choice: LineAlgorithm) -> Algorithm {
    match choice {
        LineAlgorithm::Patience => Algorithm::Patience,
        LineAlgorithm::Histogram => Algorithm::Histogram,
        LineAlgorithm::Myers => Algorithm::Myers,
    }
}

/// Lines up `before` and `after`, in order: stretches that cover both
/// sequences from start to end, same and different in turn, with every run
/// of differences joined into one stretch.
pub(crate) fn align(
    before: &[u32],
    after: &[u32],
    choice: LineAlgorithm,
    deadline: Option<Instant>,
    cancel: &CancelToken,
) -> Result<Vec<Stretch>, Stopped> {
    let mut hook = Guard {
        inner: Compact::new(Replace::new(Capture::new()), before, after),
        cancel,
    };
    diff_deadline(
        algorithm(choice),
        &mut hook,
        before,
        0..before.len(),
        after,
        0..after.len(),
        deadline,
    )?;
    let ops = hook.inner.into_inner().into_inner().into_ops();
    Ok(stretches(&ops, before.len(), after.len()))
}

/// The ops as stretches, adjacent differences joined, with no gaps.
fn stretches(ops: &[DiffOp], before_len: usize, after_len: usize) -> Vec<Stretch> {
    let mut out: Vec<Stretch> = Vec::with_capacity(ops.len());
    for op in ops {
        let (before, after) = (op.old_range(), op.new_range());
        if before.is_empty() && after.is_empty() {
            continue;
        }
        let same = matches!(op, DiffOp::Equal { .. });
        match out.last_mut() {
            Some(last) if last.same == same => {
                last.before.end = before.end;
                last.after.end = after.end;
            }
            _ => out.push(Stretch {
                same,
                before,
                after,
            }),
        }
    }
    debug_assert!(
        out.last()
            .is_none_or(|last| last.before.end == before_len && last.after.end == after_len)
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(before: &[u32], after: &[u32]) -> Vec<(bool, Range<usize>, Range<usize>)> {
        align(
            before,
            after,
            LineAlgorithm::Patience,
            None,
            &CancelToken::new(),
        )
        .unwrap()
        .into_iter()
        .map(|s| (s.same, s.before, s.after))
        .collect()
    }

    #[test]
    fn stretches_cover_both_sides_in_turn() {
        assert_eq!(run(&[], &[]), []);
        assert_eq!(run(&[1, 2, 3], &[1, 2, 3]), [(true, 0..3, 0..3)]);
        assert_eq!(
            run(&[1, 2, 3], &[1, 9, 3]),
            [(true, 0..1, 0..1), (false, 1..2, 1..2), (true, 2..3, 2..3)]
        );
        assert_eq!(run(&[], &[1, 2]), [(false, 0..0, 0..2)]);
        assert_eq!(run(&[1, 2], &[]), [(false, 0..2, 0..0)]);
    }

    #[test]
    fn a_cancelled_run_stops() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let result = align(&[1, 2], &[3, 4], LineAlgorithm::Myers, None, &cancel);
        assert_eq!(result, Err(Stopped));
    }

    #[test]
    fn every_algorithm_lines_up_the_same_simple_change() {
        for choice in [
            LineAlgorithm::Patience,
            LineAlgorithm::Histogram,
            LineAlgorithm::Myers,
        ] {
            let got = align(
                &[1, 2, 3, 4],
                &[1, 2, 5, 4],
                choice,
                None,
                &CancelToken::new(),
            )
            .unwrap();
            assert_eq!(got.len(), 3, "{choice:?}");
            assert!(!got[1].same);
            assert_eq!(got[1].before, 2..3);
        }
    }
}
