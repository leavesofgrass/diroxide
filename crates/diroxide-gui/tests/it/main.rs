//! Integration tests of the window frontend's contract.

use diroxide_core::Explain;
use diroxide_gui::GuiError;

#[test]
fn the_window_says_what_to_use_instead() {
    assert_eq!(
        GuiError::NotBuilt.next_step(),
        "Use the terminal: run diro with a folder."
    );
}
