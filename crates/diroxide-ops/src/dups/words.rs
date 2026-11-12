//! Small word helpers shared by the duplicates module.

use diroxide_core::format::human_size;

/// "1.2 of 4.0 GB" when both share a unit, else "512 MB of 4.0 GB".
pub(super) fn bytes_of(done: u64, total: u64) -> String {
    let done_text = human_size(done);
    let total_text = human_size(total);
    match (done_text.rsplit_once(' '), total_text.rsplit_once(' ')) {
        (Some((number, unit)), Some((_, total_unit))) if unit == total_unit => {
            format!("{number} of {total_text}")
        }
        _ => format!("{done_text} of {total_text}"),
    }
}
