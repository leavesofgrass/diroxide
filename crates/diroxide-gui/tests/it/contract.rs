//! The GUI frontend's contract: errors in words, the options' defaults.

use diroxide_core::{Explain, Message};
use diroxide_gui::{GraphicsApi, GuiError, GuiOptions};

#[test]
fn a_gui_that_cannot_start_says_what_to_use_instead() {
    assert_eq!(
        GuiError::NotBuilt.next_step(),
        "Use the terminal with diro --tui, or build diro with its gui feature."
    );
    let said = Message::from_error(
        "open",
        "the GUI",
        &GuiError::NoDisplay("neither WAYLAND_DISPLAY nor DISPLAY is set".to_owned()),
    );
    assert_eq!(
        said.text,
        "Could not open the GUI: there is no display to open a window on (neither WAYLAND_DISPLAY nor DISPLAY is set). Use the terminal with diro --tui, or start diro from the desktop."
    );
}

#[test]
fn the_options_default_to_a_shown_window_and_the_drivers_choice() {
    let options = GuiOptions::default();
    assert_eq!(options.title, "diroxide");
    assert!(!options.background);
    assert_eq!(options.graphics, GraphicsApi::Auto);
    assert_eq!(diroxide_gui::built(), cfg!(feature = "gui"));
}
