//! The command catalog: every built-in command's id, name, help, layer and
//! default keys in both columns (terminal and window), in one table, so
//! menus, the palette, banks, F1 help and `docs/keyboard.md` all read the
//! same names and keys (ADR-0005).
//!
//! The defaults are plan section 5.7's: the CUA editing keys, Explorer's
//! and the Finder's file keys, and the Norton function-key row, with the
//! keys the owner added on Sunday, October 11, 2026 (answer 29), the sort
//! keys (answer 33), Say Status on Ctrl+D and details on Alt+Enter
//! (question 63). There are no sequences and no leader keys. Insert is
//! never bound: it is the NVDA and JAWS modifier key. A command with no
//! terminal key is marked palette-only and is reached by the palette and
//! its menu.
//!
//! Also here: the command text form shared by the banks and `diro run`
//! (an id, then `name=value` arguments), and the sort commands with the
//! Sort list.

use std::fmt;

use diroxide_core::{Command, CommandId, ListModel, ListRow, SortKey, SortSpec};

use diroxide_core::Layer;

/// The built-in menus, in menu bar order. `menus.toml` may add more; these
/// are the ones a `menu.*` command opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Menu {
    /// File.
    File,
    /// Edit.
    Edit,
    /// View.
    View,
    /// Go.
    Go,
    /// Select.
    Select,
    /// Tools.
    Tools,
    /// Plugins.
    Plugins,
    /// Help.
    Help,
}

impl Menu {
    /// Every menu, in order.
    pub const ALL: [Menu; 8] = [
        Menu::File,
        Menu::Edit,
        Menu::View,
        Menu::Go,
        Menu::Select,
        Menu::Tools,
        Menu::Plugins,
        Menu::Help,
    ];

    /// The menu's title.
    pub const fn title(self) -> &'static str {
        match self {
            Menu::File => "File",
            Menu::Edit => "Edit",
            Menu::View => "View",
            Menu::Go => "Go",
            Menu::Select => "Select",
            Menu::Tools => "Tools",
            Menu::Plugins => "Plugins",
            Menu::Help => "Help",
        }
    }

    /// The menu's id in `menus.toml`.
    pub const fn id(self) -> &'static str {
        match self {
            Menu::File => "file",
            Menu::Edit => "edit",
            Menu::View => "view",
            Menu::Go => "go",
            Menu::Select => "select",
            Menu::Tools => "tools",
            Menu::Plugins => "plugins",
            Menu::Help => "help",
        }
    }

    /// The command that opens the menu.
    pub const fn command(self) -> CommandId {
        match self {
            Menu::File => ids::MENU_FILE,
            Menu::Edit => ids::MENU_EDIT,
            Menu::View => ids::MENU_VIEW,
            Menu::Go => ids::MENU_GO,
            Menu::Select => ids::MENU_SELECT,
            Menu::Tools => ids::MENU_TOOLS,
            Menu::Plugins => ids::MENU_PLUGINS,
            Menu::Help => ids::MENU_HELP,
        }
    }
}

impl fmt::Display for Menu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

/// One built-in command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    /// The id.
    pub id: CommandId,
    /// The name in menus and the palette, meaning first.
    pub name: &'static str,
    /// One sentence for F1 help.
    pub help: &'static str,
    /// The layer its keys belong to.
    pub layer: Layer,
    /// Default keys in the terminal column, as written in `keymap.toml`.
    pub terminal: &'static [&'static str],
    /// Default keys in the window column, as written in `keymap.toml`; on
    /// macOS Control becomes Command (see the keymap).
    pub window: &'static [&'static str],
    /// No terminal key by design: the palette and the command's menu reach
    /// it.
    pub palette_only: bool,
}

/// Well-known command ids.
pub mod ids {
    use diroxide_core::CommandId;

    macro_rules! ids {
        ($($(#[$doc:meta])* $name:ident = $id:literal;)+) => {
            $($(#[$doc])* pub const $name: CommandId = CommandId::from_static($id);)+
        };
    }

    ids! {
        /// Leave diroxide.
        QUIT = "app.quit";
        /// Leave diroxide; the shell that started it moves to the active
        /// folder.
        QUIT_TO_FOLDER = "app.quit-to-folder";
        /// Leave diroxide; the shell that started it stays in its own
        /// folder, even with `--cwd-file`.
        QUIT_KEEP_FOLDER = "app.quit-keep-folder";
        /// Say help for where the focus is.
        HELP = "app.help";
        /// Say where the focus is, and any job's progress.
        SAY_STATUS = "app.say-status";
        /// Say the other panel's place and focused row.
        SAY_OTHER = "app.say-other";
        /// Say the key bar.
        SAY_KEY_BAR = "app.say-key-bar";
        /// Show the next bank on the key bar.
        SWITCH_BANK = "app.switch-bank";
        /// Open the command palette.
        PALETTE = "app.palette";
        /// Open the menus.
        MENU = "app.menu";
        /// Open the settings.
        SETTINGS = "app.settings";
        /// Say what a key sends and what it would run.
        KEY_TEST = "app.key-test";
        /// Close the menu or dialog, or stop the job in front.
        CANCEL = "app.cancel";
        /// Cycle the layouts.
        LAYOUT = "app.layout";
        /// The next theme.
        NEXT_THEME = "app.next-theme";
        /// List the config files' problems.
        PROBLEMS = "app.problems";
        /// Undo the last job.
        UNDO = "edit.undo";
        /// List the jobs that ran.
        JOB_HISTORY = "jobs.history";
        /// Open the File menu.
        MENU_FILE = "menu.file";
        /// Open the Edit menu.
        MENU_EDIT = "menu.edit";
        /// Open the View menu.
        MENU_VIEW = "menu.view";
        /// Open the Go menu.
        MENU_GO = "menu.go";
        /// Open the Select menu.
        MENU_SELECT = "menu.select";
        /// Open the Tools menu.
        MENU_TOOLS = "menu.tools";
        /// Open the Plugins menu.
        MENU_PLUGINS = "menu.plugins";
        /// Open the Help menu.
        MENU_HELP = "menu.help";
        /// Make the other panel active.
        SWITCH_PANEL = "panel.switch";
        /// Swap the panels' places.
        SWAP_PANELS = "panel.swap";
        /// Show the focused folder in the other panel.
        SHOW_IN_OTHER = "panel.show-in-other";
        /// Show this folder in the other panel.
        THIS_IN_OTHER = "panel.this-in-other";
        /// Open the focused entry.
        ENTER = "panel.enter";
        /// Enter the focused folder or archive.
        INTO = "panel.into";
        /// Open the focused file as an archive, by its bytes.
        OPEN_AS_ARCHIVE = "archive.open-as";
        /// Make a link to each selected entry in the other panel.
        MAKE_LINK = "file.link";
        /// Point the focused link elsewhere.
        RELINK = "file.relink";
        /// Change the selection's permissions or attributes.
        PERMISSIONS = "file.permissions";
        /// Copy the focused entry's full path to the system clipboard.
        COPY_PATH = "file.copy-path";
        /// Copy the focused entry's name to the system clipboard.
        COPY_NAME = "file.copy-name";
        /// Split the focused file into numbered parts.
        SPLIT = "file.split";
        /// Join numbered parts back into one file.
        JOIN = "file.join";
        /// Pack the selection into a new archive.
        PACK = "archive.pack";
        /// Unpack the focused archive.
        UNPACK = "archive.unpack";
        /// Test the focused archive.
        TEST_ARCHIVE = "archive.test";
        /// Go up one folder.
        UP = "panel.up";
        /// List the place again.
        RELOAD = "panel.reload";
        /// Back in the folder history.
        BACK = "go.back";
        /// Forward in the folder history.
        FORWARD = "go.forward";
        /// The home folder.
        HOME = "go.home";
        /// Type a path to go to.
        GO_PATH = "go.path";
        /// The go-to-place list.
        PLACES = "go.places";
        /// The folder history list.
        HISTORY = "go.history";
        /// Focus up one row.
        LIST_UP = "list.up";
        /// Focus down one row.
        LIST_DOWN = "list.down";
        /// Focus up one page.
        LIST_PAGE_UP = "list.page-up";
        /// Focus down one page.
        LIST_PAGE_DOWN = "list.page-down";
        /// Focus the first row.
        LIST_HOME = "list.home";
        /// Focus the last row.
        LIST_END = "list.end";
        /// Select or unselect the focused row and move down.
        SELECT_TOGGLE = "select.toggle";
        /// Select every row.
        SELECT_ALL = "select.all";
        /// Clear the selection.
        SELECT_NONE = "select.none";
        /// Invert the selection.
        SELECT_INVERT = "select.invert";
        /// Select by a pattern.
        SELECT_PATTERN = "select.pattern";
        /// Unselect by a pattern.
        UNSELECT_PATTERN = "select.unpattern";
        /// The next selected row.
        SELECT_NEXT = "select.next";
        /// The previous selected row.
        SELECT_PREVIOUS = "select.previous";
        /// Bring back the selection the last wholesale change replaced.
        SELECT_RESTORE = "select.restore";
        /// View the focused file.
        VIEW = "file.view";
        /// Edit the focused file.
        EDIT = "file.edit";
        /// Copy to the other panel.
        COPY = "file.copy";
        /// Move to the other panel.
        MOVE = "file.move";
        /// Copy to a place chosen from a list.
        COPY_TO = "file.copy-to";
        /// Move to a place chosen from a list.
        MOVE_TO = "file.move-to";
        /// Make a folder.
        MAKE_FOLDER = "folder.make";
        /// Move to the trash.
        TRASH = "file.trash";
        /// Delete for good, after listing every path.
        DELETE_PERMANENTLY = "file.delete-permanently";
        /// Rename the focused entry.
        RENAME = "file.rename";
        /// Duplicate in this folder.
        DUPLICATE = "file.duplicate";
        /// The details of the focused entry.
        DETAILS = "file.details";
        /// The context menu of the focused entry.
        CONTEXT_MENU = "file.context-menu";
        /// Run a command line in this folder.
        RUN_COMMAND = "file.run-command";
        /// The shell in this folder.
        SHELL_HERE = "file.shell-here";
        /// Copy to the file clipboard.
        CLIPBOARD_COPY = "clipboard.copy";
        /// Cut to the file clipboard.
        CLIPBOARD_CUT = "clipboard.cut";
        /// Paste from the file clipboard.
        CLIPBOARD_PASTE = "clipboard.paste";
        /// Show or hide hidden entries.
        TOGGLE_HIDDEN = "view.hidden";
        /// Filter the names shown.
        FILTER = "view.filter";
        /// Clear the filter.
        FILTER_CLEAR = "view.filter-clear";
        /// Sort by name.
        SORT_NAME = "view.sort-name";
        /// Sort by extension.
        SORT_EXTENSION = "view.sort-extension";
        /// Sort by modified time.
        SORT_MODIFIED = "view.sort-modified";
        /// Sort by size.
        SORT_SIZE = "view.sort-size";
        /// The folder's own order.
        SORT_UNSORTED = "view.sort-unsorted";
        /// Sort by created time.
        SORT_CREATED = "view.sort-created";
        /// Sort by accessed time.
        SORT_ACCESSED = "view.sort-accessed";
        /// Sort by kind.
        SORT_KIND = "view.sort-kind";
        /// Sort by attributes or permissions.
        SORT_ATTRIBUTES = "view.sort-attributes";
        /// Sort by owner.
        SORT_OWNER = "view.sort-owner";
        /// Sort by label.
        SORT_LABEL = "view.sort-label";
        /// The Sort list.
        SORT_LIST = "view.sort-list";
        /// Closes the viewer and goes back to the file's row
        CLOSE_VIEWER = "viewer.close";
        /// Moves to the token before, or the byte before in hex mode, and says it
        VIEWER_LEFT = "viewer.left";
        /// Moves to the next token, or the next byte in hex mode, and says it
        VIEWER_RIGHT = "viewer.right";
        /// Wraps long lines at the screen's width, or stops wrapping
        VIEWER_WRAP = "viewer.wrap";
        /// Shows or hides the number of each line
        VIEWER_LINE_NUMBERS = "viewer.line-numbers";
        /// Asks for a line, an offset or a percent, and goes there
        VIEWER_JUMP = "viewer.jump";
        /// Asks for text, quoted text or hex bytes, and finds it after the cursor
        VIEWER_SEARCH = "viewer.search";
        /// Asks for text and finds it before the cursor
        VIEWER_SEARCH_BACK = "viewer.search-back";
        /// Goes to the next match of the last search
        VIEWER_NEXT_MATCH = "viewer.next-match";
        /// Goes to the match before, of the last search
        VIEWER_PREVIOUS_MATCH = "viewer.previous-match";
        /// Switches between the text and rows of bytes
        VIEWER_HEX = "viewer.hex";
        /// Says the byte at the cursor in hex and as a character
        VIEWER_BYTE = "viewer.byte";
        /// Reads further into a file larger than the viewer keeps
        VIEWER_READ_MORE = "viewer.read-more";
        /// Reads the file again from its place
        VIEWER_RELOAD = "viewer.reload";
        /// Goes to the end and stays there as the file grows, or stops
        VIEWER_FOLLOW = "viewer.follow";
        /// Shows the next file of the folder in the viewer
        VIEWER_NEXT_FILE = "viewer.next-file";
        /// Shows the file before in the viewer
        VIEWER_PREVIOUS_FILE = "viewer.previous-file";
        /// Closes the viewer with the file's row focused in its folder
        VIEWER_SHOW_IN_FOLDER = "viewer.show-in-folder";
        /// Puts the viewed file's full path on the system clipboard
        VIEWER_COPY_PATH = "viewer.copy-path";
        /// Puts the viewed file's name on the system clipboard
        VIEWER_COPY_NAME = "viewer.copy-name";
        /// Says what the token at the cursor is, such as a keyword or a function name
        VIEWER_TOKEN_KIND = "viewer.token-kind";
        /// Says the kinds of token on the cursor's line, in order
        VIEWER_LINE_KINDS = "viewer.line-kinds";
        /// Lists the file's symbols, to go to one
        VIEWER_OUTLINE = "viewer.outline";
        /// Goes to the next function, type or heading
        VIEWER_NEXT_SYMBOL = "viewer.next-symbol";
        /// Goes to the function, type or heading before
        VIEWER_PREVIOUS_SYMBOL = "viewer.previous-symbol";
        /// Goes to the bracket that matches the one at the cursor
        VIEWER_BRACKET = "viewer.bracket";
        /// Goes to the next line of code after a comment
        VIEWER_SKIP_COMMENT = "viewer.skip-comment";
        /// Says the indent level of the cursor's line
        VIEWER_INDENT = "viewer.indent";
        /// Goes to the next line at the same indent, within the block
        VIEWER_INDENT_NEXT = "viewer.indent-next";
        /// Goes to the line before at the same indent, within the block
        VIEWER_INDENT_PREVIOUS = "viewer.indent-previous";
        /// Goes to the line before with less indent
        VIEWER_INDENT_PARENT = "viewer.indent-parent";
        /// Goes to the next line with more indent, inside this one
        VIEWER_INDENT_CHILD = "viewer.indent-child";
        /// Says the function and the blocks the cursor is in
        VIEWER_CONTEXT = "viewer.context";
        /// Turns the other panel into a live preview of the file under the cursor, or back
        QUICK_VIEW = "view.quick-view";
        /// Hands the quick view's file to textweaver for full reading with speech
        OPEN_IN_TEXTWEAVER = "preview.textweaver";
        /// Goes back to the file panel, the cursor on its row
        PREVIEW_LEAVE = "preview.leave";
        /// Goes to the next heading below, its level said
        PREVIEW_NEXT_HEADING = "preview.next-heading";
        /// Goes to the heading above, its level said
        PREVIEW_PREVIOUS_HEADING = "preview.previous-heading";
        /// Goes to the next paragraph; a table or a code block is one
        PREVIEW_NEXT_PARAGRAPH = "preview.next-paragraph";
        /// Goes to the paragraph above, or the start of this one
        PREVIEW_PREVIOUS_PARAGRAPH = "preview.previous-paragraph";
        /// Goes to the next table, its size said
        PREVIEW_NEXT_TABLE = "preview.next-table";
        /// Goes to the table above, its size said
        PREVIEW_PREVIOUS_TABLE = "preview.previous-table";
        /// In a table, the cell to the left, said with its header
        PREVIEW_CELL_LEFT = "preview.cell-left";
        /// In a table, the cell to the right, said with its header
        PREVIEW_CELL_RIGHT = "preview.cell-right";
        /// In a table, the cell above, said with its header
        PREVIEW_CELL_UP = "preview.cell-up";
        /// In a table, the cell below, said with its header
        PREVIEW_CELL_DOWN = "preview.cell-down";
        /// Lists the preview's links, to go to one
        PREVIEW_LINKS = "preview.links";
        /// Goes to the next link, said with where it goes
        PREVIEW_NEXT_LINK = "preview.next-link";
        /// Goes to the link before, said with where it goes
        PREVIEW_PREVIOUS_LINK = "preview.previous-link";
        /// Opens the web or mail address of the link on this line, asked first
        PREVIEW_OPEN_LINK = "preview.open-link";
        /// Asks for text and finds it in the preview, the count said
        PREVIEW_FIND = "preview.find";
        /// Goes to the next match of the last find in the preview
        PREVIEW_NEXT_MATCH = "preview.next-match";
        /// Goes to the match before, of the last find in the preview
        PREVIEW_PREVIOUS_MATCH = "preview.previous-match";
        /// Shows the panel's next file in the preview without leaving it
        PREVIEW_NEXT_FILE = "preview.next-file";
        /// Shows the panel's file before in the preview without leaving it
        PREVIEW_PREVIOUS_FILE = "preview.previous-file";
        /// List the plugins.
        PLUGINS = "plugins.list";
        /// Yes, in a question.
        YES = "dialog.yes";
        /// No, in a question.
        NO = "dialog.no";
        /// The focused answer, in a dialog.
        ACCEPT = "dialog.accept";
        /// Change a command's key (W3k): with a `command` argument, the
        /// capture waits for that command's new key; without one, the Keys
        /// list opens.
        KEYS_CHANGE = "keys.change";
        /// The Keys list (W3k).
        KEYS_LIST = "keys.list";
        /// Reset all keys (W3k).
        KEYS_RESET_ALL = "keys.reset-all";
        /// The single-letter switch (W3k).
        KEYS_SINGLE_LETTERS = "keys.single-letters";
    }
}

use Layer::{Dialog, Global, List, Panel, Preview, Viewer};

const fn command(
    id: CommandId,
    name: &'static str,
    help: &'static str,
    layer: Layer,
    terminal: &'static [&'static str],
    window: &'static [&'static str],
) -> CommandInfo {
    CommandInfo {
        id,
        name,
        help,
        layer,
        terminal,
        window,
        palette_only: terminal.is_empty(),
    }
}

/// Every built-in command, in the order the keyboard page lists them.
pub static CATALOG: [CommandInfo; 162] = [
    command(
        ids::QUIT,
        "Quit",
        "Leaves diroxide.",
        Global,
        &["ctrl+q"],
        &["ctrl+q"],
    ),
    command(
        ids::QUIT_TO_FOLDER,
        "Quit to this folder",
        "Leaves diroxide, and the shell that started it moves to the active folder.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::QUIT_KEEP_FOLDER,
        "Quit, shell stays where it was",
        "Leaves diroxide, and the shell that started it stays in its own folder.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::HELP,
        "Help",
        "Says help for where the focus is.",
        Global,
        &["shift+f1", "?"],
        &["f1"],
    ),
    command(
        ids::SAY_STATUS,
        "Say status",
        "Says the panel, the count, the selection, the focus and any job's progress.",
        Global,
        &["ctrl+d"],
        &["ctrl+d"],
    ),
    command(
        ids::SAY_OTHER,
        "Say other panel",
        "Says the other panel's place and its focused row.",
        Global,
        &["alt+o"],
        &["alt+o"],
    ),
    command(
        ids::SAY_KEY_BAR,
        "Say key bar",
        "Says the function keys and what each one does.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::SWITCH_BANK,
        "Switch bank",
        "Shows the next bank of buttons on the key bar.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::PALETTE,
        "Command palette",
        "Finds any command by name and runs it.",
        Global,
        &["ctrl+p"],
        &["ctrl+shift+p", "ctrl+p"],
    ),
    command(
        ids::MENU,
        "Menus",
        "Opens the menu bar.",
        Global,
        &["f9"],
        &["f9", "f10"],
    ),
    command(
        ids::SETTINGS,
        "Settings",
        "Lists the settings; Help on a setting says what it does.",
        Global,
        &[],
        &["ctrl+,"],
    ),
    command(
        ids::KEY_TEST,
        "Test keys",
        "Says what each key you press sends and what it would run; Cancel ends it.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::CANCEL,
        "Cancel",
        "Closes the menu or dialog, or stops the job in front.",
        Global,
        &["escape"],
        &["escape"],
    ),
    command(
        ids::LAYOUT,
        "Change layout",
        "Moves between side by side, stacked and single panel.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::NEXT_THEME,
        "Next theme",
        "Switches to the next theme and says its contrast.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::PROBLEMS,
        "Config problems",
        "Lists what could not be read from the config files, and why.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::UNDO,
        "Undo",
        "Reverses the last job: a rename, move, copy, new folder or trashing.",
        Global,
        &["ctrl+z"],
        &["ctrl+z"],
    ),
    command(
        ids::JOB_HISTORY,
        "Job history",
        "Lists the jobs that ran, what failed in each, and why.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::MENU_FILE,
        "File menu",
        "Opens the File menu.",
        Global,
        &["alt+f"],
        &["alt+f"],
    ),
    command(
        ids::MENU_EDIT,
        "Edit menu",
        "Opens the Edit menu.",
        Global,
        &["alt+e"],
        &["alt+e"],
    ),
    command(
        ids::MENU_VIEW,
        "View menu",
        "Opens the View menu.",
        Global,
        &["alt+v"],
        &["alt+v"],
    ),
    command(
        ids::MENU_GO,
        "Go menu",
        "Opens the Go menu.",
        Global,
        &["alt+g"],
        &["alt+g"],
    ),
    command(
        ids::MENU_SELECT,
        "Select menu",
        "Opens the Select menu.",
        Global,
        &["alt+s"],
        &["alt+s"],
    ),
    command(
        ids::MENU_TOOLS,
        "Tools menu",
        "Opens the Tools menu.",
        Global,
        &["alt+t"],
        &["alt+t"],
    ),
    command(
        ids::MENU_PLUGINS,
        "Plugins menu",
        "Opens the Plugins menu.",
        Global,
        &["alt+p"],
        &["alt+p"],
    ),
    command(
        ids::MENU_HELP,
        "Help menu",
        "Opens the Help menu.",
        Global,
        &["alt+h"],
        &["alt+h"],
    ),
    command(
        ids::SWITCH_PANEL,
        "Switch panels",
        "Makes the other panel active.",
        Panel,
        &["tab"],
        &["tab"],
    ),
    command(
        ids::SWAP_PANELS,
        "Swap panels",
        "Swaps the places the two panels show.",
        Panel,
        &["ctrl+u"],
        &["ctrl+u"],
    ),
    command(
        ids::SHOW_IN_OTHER,
        "Folder to other panel",
        "Shows the focused folder in the other panel.",
        Panel,
        &["ctrl+right"],
        &["ctrl+right"],
    ),
    command(
        ids::THIS_IN_OTHER,
        "This folder to other panel",
        "Shows this folder in the other panel too.",
        Panel,
        &["ctrl+left"],
        &["ctrl+left"],
    ),
    command(
        ids::ENTER,
        "Open",
        "Opens the focused folder, archive or file.",
        Panel,
        &["enter"],
        &["enter"],
    ),
    command(
        ids::INTO,
        "Into folder",
        "Enters the focused folder or archive; on a file, says its kind.",
        Panel,
        &["right"],
        &["right"],
    ),
    command(
        ids::OPEN_AS_ARCHIVE,
        "Open as archive",
        "Opens the focused file as an archive by what its bytes say, whatever its name.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::MAKE_LINK,
        "Make symbolic link",
        "Makes a link to each selected entry in the other panel, after a list of its settings.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::RELINK,
        "Change link target",
        "Points the focused link to another place; Undo points it back.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::PERMISSIONS,
        "Change permissions",
        "Changes who may read, write and run the selection, after a list of its settings.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::COPY_PATH,
        "Copy path",
        "Puts the focused entry's full path on the system clipboard.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::COPY_NAME,
        "Copy name",
        "Puts the focused entry's name on the system clipboard.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SPLIT,
        "Split file",
        "Splits the focused file, or each marked file, into numbered parts in the other panel.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::JOIN,
        "Join files",
        "Joins numbered parts back into one file in the other panel, from any part.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::PACK,
        "Pack",
        "Packs the selection into a new archive in the other panel, after a list of its settings.",
        Panel,
        &[],
        &["alt+f5"],
    ),
    command(
        ids::UNPACK,
        "Unpack",
        "Unpacks the focused archive into the other panel, after a list of its settings.",
        Panel,
        &[],
        &["alt+f9"],
    ),
    command(
        ids::TEST_ARCHIVE,
        "Test archive",
        "Reads every member of the focused archive and checks it, writing nothing.",
        Panel,
        &[],
        &["alt+shift+f9"],
    ),
    command(
        ids::UP,
        "Up one folder",
        "Goes to the folder above, with the cursor on the folder just left.",
        Panel,
        &["left", "backspace", "alt+up"],
        &["left", "backspace", "alt+up"],
    ),
    command(
        ids::RELOAD,
        "Reload",
        "Lists the folder again.",
        Panel,
        &["ctrl+r"],
        &["ctrl+r"],
    ),
    command(
        ids::BACK,
        "Back",
        "Goes to the folder before this one, with its cursor and marks.",
        Panel,
        &["alt+left"],
        &["alt+left"],
    ),
    command(
        ids::FORWARD,
        "Forward",
        "Goes forward again after Back.",
        Panel,
        &["alt+right"],
        &["alt+right"],
    ),
    command(
        ids::HOME,
        "Home folder",
        "Goes to your home folder.",
        Panel,
        &["alt+home"],
        &["alt+home"],
    ),
    command(
        ids::GO_PATH,
        "Go to a path",
        "Asks for a path and goes there, completing names as you type.",
        Panel,
        &["ctrl+l", "alt+d"],
        &["ctrl+l", "alt+d", "ctrl+enter"],
    ),
    command(
        ids::PLACES,
        "Go to a place",
        "Lists the drives, home and the places you use most.",
        Panel,
        &["ctrl+g"],
        &["ctrl+g"],
    ),
    command(
        ids::HISTORY,
        "Folder history",
        "Lists the folders visited, newest first.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::LIST_UP,
        "Previous row",
        "Moves the focus up one row.",
        List,
        &["up"],
        &["up"],
    ),
    command(
        ids::LIST_DOWN,
        "Next row",
        "Moves the focus down one row.",
        List,
        &["down"],
        &["down"],
    ),
    command(
        ids::LIST_PAGE_UP,
        "Previous page",
        "Moves the focus up one page.",
        List,
        &["pageup"],
        &["pageup"],
    ),
    command(
        ids::LIST_PAGE_DOWN,
        "Next page",
        "Moves the focus down one page.",
        List,
        &["pagedown"],
        &["pagedown"],
    ),
    command(
        ids::LIST_HOME,
        "First row",
        "Moves the focus to the first row.",
        List,
        &["home"],
        &["home"],
    ),
    command(
        ids::LIST_END,
        "Last row",
        "Moves the focus to the last row.",
        List,
        &["end"],
        &["end"],
    ),
    command(
        ids::SELECT_TOGGLE,
        "Select or unselect",
        "Selects or unselects the focused row and moves down.",
        Panel,
        &["space"],
        &["space"],
    ),
    command(
        ids::SELECT_ALL,
        "Select all",
        "Selects every row.",
        Panel,
        &["ctrl+a"],
        &["ctrl+a", "kp-plus"],
    ),
    command(
        ids::SELECT_NONE,
        "Select none",
        "Clears the selection.",
        Panel,
        &[],
        &["ctrl+shift+a", "kp-minus"],
    ),
    command(
        ids::SELECT_INVERT,
        "Invert selection",
        "Selects what was not selected, and the reverse.",
        Panel,
        &["*"],
        &["*", "kp-multiply"],
    ),
    command(
        ids::SELECT_PATTERN,
        "Select by pattern",
        "Selects the rows whose names match a pattern; it starts as a star, which selects all.",
        Panel,
        &["plus"],
        &["plus", "kp-divide"],
    ),
    command(
        ids::UNSELECT_PATTERN,
        "Unselect by pattern",
        "Unselects the rows whose names match a pattern.",
        Panel,
        &["-"],
        &["-"],
    ),
    command(
        ids::SELECT_NEXT,
        "Next selected",
        "Moves to the next selected row.",
        Panel,
        &["ctrl+down"],
        &["ctrl+down"],
    ),
    command(
        ids::SELECT_PREVIOUS,
        "Previous selected",
        "Moves to the previous selected row.",
        Panel,
        &["ctrl+up"],
        &["ctrl+up"],
    ),
    command(
        ids::SELECT_RESTORE,
        "Restore selection",
        "Brings back the selection the last select all, none, invert, pattern, finished job or folder change replaced.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::VIEW,
        "View",
        "Opens the focused file in the viewer.",
        Panel,
        &["f3"],
        &["f3"],
    ),
    command(
        ids::EDIT,
        "Edit",
        "Opens the focused file in your editor, then comes back.",
        Panel,
        &["f4"],
        &["f4"],
    ),
    command(
        ids::COPY,
        "Copy to other panel",
        "Copies the selection to the other panel's folder.",
        Panel,
        &["f5"],
        &["f5"],
    ),
    command(
        ids::MOVE,
        "Move to other panel",
        "Moves the selection to the other panel's folder, after listing it.",
        Panel,
        &["f6"],
        &["f6"],
    ),
    command(
        ids::COPY_TO,
        "Copy to a place",
        "Copies the selection to a place you choose, recent places first.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::MOVE_TO,
        "Move to a place",
        "Moves the selection to a place you choose, recent places first.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::MAKE_FOLDER,
        "Make folder",
        "Asks for a name and makes a folder here.",
        Panel,
        &["f7"],
        &["f7", "ctrl+shift+n"],
    ),
    command(
        ids::TRASH,
        "Move to trash",
        "Moves the selection to the trash, after listing it.",
        Panel,
        &["f8", "delete"],
        &["f8", "delete"],
    ),
    command(
        ids::DELETE_PERMANENTLY,
        "Delete for good",
        "Deletes the selection for good after listing every path; it cannot be undone.",
        Panel,
        &["shift+delete"],
        &["shift+delete"],
    ),
    command(
        ids::RENAME,
        "Rename",
        "Asks for a new name; with several selected, steps through them.",
        Panel,
        &["f2"],
        &["f2"],
    ),
    command(
        ids::DUPLICATE,
        "Duplicate",
        "Copies the selection into this folder under new names.",
        Panel,
        &["shift+f5"],
        &["shift+f5"],
    ),
    command(
        ids::DETAILS,
        "Details",
        "Says everything known about the focused entry.",
        Panel,
        &["alt+enter"],
        &["alt+enter"],
    ),
    command(
        ids::CONTEXT_MENU,
        "Context menu",
        "Lists what can be done with the focused entry or the selection.",
        Panel,
        &["shift+f10", "menu"],
        &["shift+f10", "menu", "ctrl+space"],
    ),
    command(
        ids::RUN_COMMAND,
        "Run a command",
        "Asks for a command line and runs it in this folder.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SHELL_HERE,
        "Shell here",
        "Opens your shell in this folder; exit it to come back.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::CLIPBOARD_COPY,
        "Copy",
        "Puts the selection on the file clipboard, to paste elsewhere.",
        Panel,
        &["ctrl+c"],
        &["ctrl+c"],
    ),
    command(
        ids::CLIPBOARD_CUT,
        "Cut",
        "Puts the selection on the file clipboard, to move when pasted.",
        Panel,
        &["ctrl+x"],
        &["ctrl+x"],
    ),
    command(
        ids::CLIPBOARD_PASTE,
        "Paste",
        "Copies or moves what is on the file clipboard into this folder.",
        Panel,
        &["ctrl+v"],
        &["ctrl+v"],
    ),
    command(
        ids::TOGGLE_HIDDEN,
        "Show hidden files",
        "Shows or hides hidden files and folders.",
        Panel,
        &["alt+."],
        &["alt+."],
    ),
    command(
        ids::FILTER,
        "Filter",
        "Shows only the names that match what you type; Cancel or Clear filter ends it.",
        Panel,
        &["ctrl+s"],
        &["ctrl+s"],
    ),
    command(
        ids::FILTER_CLEAR,
        "Clear filter",
        "Shows every name again.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_NAME,
        "Sort by name",
        "Sorts by name, A to Z; again for Z to A.",
        Panel,
        &["ctrl+f3"],
        &["ctrl+f3"],
    ),
    command(
        ids::SORT_EXTENSION,
        "Sort by extension",
        "Sorts by extension, A to Z; again for Z to A.",
        Panel,
        &["ctrl+f4"],
        &["ctrl+f4"],
    ),
    command(
        ids::SORT_MODIFIED,
        "Sort by modified",
        "Sorts by modified time, newest first; again for oldest first.",
        Panel,
        &["ctrl+f5"],
        &["ctrl+f5"],
    ),
    command(
        ids::SORT_SIZE,
        "Sort by size",
        "Sorts by size, largest first; again for smallest first.",
        Panel,
        &["ctrl+f6"],
        &["ctrl+f6"],
    ),
    command(
        ids::SORT_UNSORTED,
        "Unsorted",
        "Lists in the folder's own order.",
        Panel,
        &["ctrl+f7"],
        &["ctrl+f7"],
    ),
    command(
        ids::SORT_CREATED,
        "Sort by created",
        "Sorts by created time, newest first; again for oldest first.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_ACCESSED,
        "Sort by accessed",
        "Sorts by accessed time, newest first; again for oldest first.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_KIND,
        "Sort by kind",
        "Sorts by the kind of file, A to Z; again for Z to A.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_ATTRIBUTES,
        "Sort by attributes",
        "Sorts by permissions or attributes; again to reverse.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_OWNER,
        "Sort by owner",
        "Sorts by owner, A to Z; again for Z to A.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_LABEL,
        "Sort by label",
        "Sorts by label, A to Z; again for Z to A.",
        Panel,
        &[],
        &[],
    ),
    command(
        ids::SORT_LIST,
        "Sort list",
        "Lists every sort key, the current one marked with its direction.",
        Panel,
        &[],
        &["ctrl+shift+s"],
    ),
    command(
        ids::CLOSE_VIEWER,
        "Close viewer",
        "Closes the viewer and goes back to the file's row.",
        Viewer,
        &["escape", "q"],
        &["escape", "q"],
    ),
    command(
        ids::VIEWER_LEFT,
        "Previous token",
        "Moves to the token before, or the byte before in hex mode, and says it.",
        Viewer,
        &["left"],
        &["left"],
    ),
    command(
        ids::VIEWER_RIGHT,
        "Next token",
        "Moves to the next token, or the next byte in hex mode, and says it.",
        Viewer,
        &["right"],
        &["right"],
    ),
    command(
        ids::VIEWER_WRAP,
        "Wrap lines",
        "Wraps long lines at the screen's width, or stops wrapping.",
        Viewer,
        &["w"],
        &["w"],
    ),
    command(
        ids::VIEWER_LINE_NUMBERS,
        "Line numbers",
        "Shows or hides the number of each line.",
        Viewer,
        &["l"],
        &["l"],
    ),
    command(
        ids::VIEWER_JUMP,
        "Go to line",
        "Asks for a line, an offset or a percent, and goes there.",
        Viewer,
        &["g", "ctrl+g"],
        &["g", "ctrl+g"],
    ),
    command(
        ids::VIEWER_SEARCH,
        "Search in file",
        "Asks for text, quoted text or hex bytes, and finds it after the cursor.",
        Viewer,
        &["/", "f7"],
        &["/", "f7"],
    ),
    command(
        ids::VIEWER_SEARCH_BACK,
        "Search backward",
        "Asks for text and finds it before the cursor.",
        Viewer,
        &["?"],
        &["?"],
    ),
    command(
        ids::VIEWER_NEXT_MATCH,
        "Next match",
        "Goes to the next match of the last search.",
        Viewer,
        &["n"],
        &["n"],
    ),
    command(
        ids::VIEWER_PREVIOUS_MATCH,
        "Previous match",
        "Goes to the match before, of the last search.",
        Viewer,
        &["p"],
        &["p"],
    ),
    command(
        ids::VIEWER_HEX,
        "Hex mode",
        "Switches between the text and rows of bytes.",
        Viewer,
        &["h"],
        &["h"],
    ),
    command(
        ids::VIEWER_BYTE,
        "Say byte",
        "Says the byte at the cursor in hex and as a character.",
        Viewer,
        &["b"],
        &["b"],
    ),
    command(
        ids::VIEWER_READ_MORE,
        "Read more",
        "Reads further into a file larger than the viewer keeps.",
        Viewer,
        &["shift+r"],
        &["shift+r"],
    ),
    command(
        ids::VIEWER_RELOAD,
        "Reload file",
        "Reads the file again from its place.",
        Viewer,
        &["ctrl+r"],
        &["ctrl+r"],
    ),
    command(
        ids::VIEWER_FOLLOW,
        "Follow file",
        "Goes to the end and stays there as the file grows, or stops.",
        Viewer,
        &["f"],
        &["f"],
    ),
    command(
        ids::VIEWER_NEXT_FILE,
        "Next file",
        "Shows the next file of the folder in the viewer.",
        Viewer,
        &["alt+n"],
        &["alt+n"],
    ),
    command(
        ids::VIEWER_PREVIOUS_FILE,
        "Previous file",
        "Shows the file before in the viewer.",
        Viewer,
        &["alt+p"],
        &["alt+p"],
    ),
    command(
        ids::VIEWER_SHOW_IN_FOLDER,
        "Show in folder",
        "Closes the viewer with the file's row focused in its folder.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_COPY_PATH,
        "Copy viewed file's path",
        "Puts the viewed file's full path on the system clipboard.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_COPY_NAME,
        "Copy viewed file's name",
        "Puts the viewed file's name on the system clipboard.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_TOKEN_KIND,
        "Say token kind",
        "Says what the token at the cursor is, such as a keyword or a function name.",
        Viewer,
        &["k"],
        &["k"],
    ),
    command(
        ids::VIEWER_LINE_KINDS,
        "Say line by kind",
        "Says the kinds of token on the cursor's line, in order.",
        Viewer,
        &["shift+k"],
        &["shift+k"],
    ),
    command(
        ids::VIEWER_OUTLINE,
        "Outline",
        "Lists the file's symbols, to go to one.",
        Viewer,
        &["o"],
        &["o"],
    ),
    command(
        ids::VIEWER_NEXT_SYMBOL,
        "Next symbol",
        "Goes to the next function, type or heading.",
        Viewer,
        &["s"],
        &["s"],
    ),
    command(
        ids::VIEWER_PREVIOUS_SYMBOL,
        "Previous symbol",
        "Goes to the function, type or heading before.",
        Viewer,
        &["shift+s"],
        &["shift+s"],
    ),
    command(
        ids::VIEWER_BRACKET,
        "Matching bracket",
        "Goes to the bracket that matches the one at the cursor.",
        Viewer,
        &["m"],
        &["m"],
    ),
    command(
        ids::VIEWER_SKIP_COMMENT,
        "Skip comment",
        "Goes to the next line of code after a comment.",
        Viewer,
        &["c"],
        &["c"],
    ),
    command(
        ids::VIEWER_INDENT,
        "Say indent",
        "Says the indent level of the cursor's line.",
        Viewer,
        &["i"],
        &["i"],
    ),
    command(
        ids::VIEWER_INDENT_NEXT,
        "Next at this indent",
        "Goes to the next line at the same indent, within the block.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_INDENT_PREVIOUS,
        "Previous at this indent",
        "Goes to the line before at the same indent, within the block.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_INDENT_PARENT,
        "Up an indent level",
        "Goes to the line before with less indent.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_INDENT_CHILD,
        "Into the next indent",
        "Goes to the next line with more indent, inside this one.",
        Viewer,
        &[],
        &[],
    ),
    command(
        ids::VIEWER_CONTEXT,
        "Say context",
        "Says the function and the blocks the cursor is in.",
        Viewer,
        &["x"],
        &["x"],
    ),
    command(
        ids::QUICK_VIEW,
        "Quick view",
        "Turns the other panel into a live preview of the file under the cursor, or back to its files.",
        Global,
        &["shift+f3"],
        &["shift+f3"],
    ),
    command(
        ids::OPEN_IN_TEXTWEAVER,
        "Open in textweaver",
        "Hands the quick view's file to textweaver for full reading with speech, when it is installed.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::PREVIEW_LEAVE,
        "Leave the preview",
        "Goes back to the file panel, the cursor on its row.",
        Preview,
        &["escape", "tab"],
        &["escape", "tab"],
    ),
    command(
        ids::PREVIEW_NEXT_HEADING,
        "Next heading",
        "Goes to the next heading below, its level said.",
        Preview,
        &["h"],
        &["h"],
    ),
    command(
        ids::PREVIEW_PREVIOUS_HEADING,
        "Previous heading",
        "Goes to the heading above, its level said.",
        Preview,
        &["shift+h"],
        &["shift+h"],
    ),
    command(
        ids::PREVIEW_NEXT_PARAGRAPH,
        "Next paragraph",
        "Goes to the next paragraph; a table or a code block is one.",
        Preview,
        &["ctrl+down"],
        &["ctrl+down"],
    ),
    command(
        ids::PREVIEW_PREVIOUS_PARAGRAPH,
        "Previous paragraph",
        "Goes to the paragraph above, or the start of this one.",
        Preview,
        &["ctrl+up"],
        &["ctrl+up"],
    ),
    command(
        ids::PREVIEW_NEXT_TABLE,
        "Next table",
        "Goes to the next table, its rows and columns said; the cell moves start on its first cell.",
        Preview,
        &["t"],
        &["t"],
    ),
    command(
        ids::PREVIEW_PREVIOUS_TABLE,
        "Previous table",
        "Goes to the table above, its rows and columns said.",
        Preview,
        &["shift+t"],
        &["shift+t"],
    ),
    command(
        ids::PREVIEW_CELL_LEFT,
        "Cell to the left",
        "In a table, the cell to the left, said with its header.",
        Preview,
        &["left"],
        &["left"],
    ),
    command(
        ids::PREVIEW_CELL_RIGHT,
        "Cell to the right",
        "In a table, the cell to the right, said with its header.",
        Preview,
        &["right"],
        &["right"],
    ),
    command(
        ids::PREVIEW_CELL_UP,
        "Cell above",
        "In a table, the cell above, said with its header.",
        Preview,
        &["shift+up"],
        &["shift+up"],
    ),
    command(
        ids::PREVIEW_CELL_DOWN,
        "Cell below",
        "In a table, the cell below, said with its header.",
        Preview,
        &["shift+down"],
        &["shift+down"],
    ),
    command(
        ids::PREVIEW_LINKS,
        "Links in the preview",
        "Lists the preview's links, its text first and where it goes after, to go to one.",
        Preview,
        &["l"],
        &["l"],
    ),
    command(
        ids::PREVIEW_NEXT_LINK,
        "Next link",
        "Goes to the next link, said with where it goes.",
        Preview,
        &["shift+l"],
        &["shift+l"],
    ),
    command(
        ids::PREVIEW_PREVIOUS_LINK,
        "Previous link",
        "Goes to the link before, said with where it goes.",
        Preview,
        &["ctrl+l"],
        &["ctrl+l"],
    ),
    command(
        ids::PREVIEW_OPEN_LINK,
        "Open link",
        "Opens the web or mail address of the link on this line in the browser or the mail program, asked first; never a file.",
        Preview,
        &["enter"],
        &["enter"],
    ),
    command(
        ids::PREVIEW_FIND,
        "Find in the preview",
        "Asks for text and finds it in the preview, ignoring case, the count said.",
        Preview,
        &["/"],
        &["/"],
    ),
    command(
        ids::PREVIEW_NEXT_MATCH,
        "Next match in the preview",
        "Goes to the next match of the last find in the preview.",
        Preview,
        &["n"],
        &["n"],
    ),
    command(
        ids::PREVIEW_PREVIOUS_MATCH,
        "Previous match in the preview",
        "Goes to the match before, of the last find in the preview.",
        Preview,
        &["p"],
        &["p"],
    ),
    command(
        ids::PREVIEW_NEXT_FILE,
        "Next file in the preview",
        "Shows the panel's next file in the preview without leaving it; the panel's cursor follows.",
        Preview,
        &["alt+n"],
        &["alt+n"],
    ),
    command(
        ids::PREVIEW_PREVIOUS_FILE,
        "Previous file in the preview",
        "Shows the panel's file before in the preview without leaving it; the panel's cursor follows.",
        Preview,
        &["alt+p"],
        &["alt+p"],
    ),
    // Keys (W3k): Change key and the Keys list.
    command(
        ids::KEYS_LIST,
        "Keys list",
        "Lists every command with its keys in both columns; type to narrow it, and change any key.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::KEYS_RESET_ALL,
        "Reset all keys",
        "Takes every change of yours out of keymap.toml, after asking; the preset stays.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::KEYS_SINGLE_LETTERS,
        "Single letters",
        "Chooses what a key without Control or Alt does in a panel: jump to a name, run a command, or nothing.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::KEYS_CHANGE,
        "Change a key",
        "Waits for a command's new key, says it back, and says what it already does.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::PLUGINS,
        "Plugins",
        "Lists the plugins and what they may do. Plugins come with alpha.4.",
        Global,
        &[],
        &[],
    ),
    command(
        ids::YES,
        "Yes",
        "Answers yes to the question.",
        Dialog,
        &["y"],
        &["y"],
    ),
    command(
        ids::NO,
        "No",
        "Answers no to the question.",
        Dialog,
        &["n"],
        &["n"],
    ),
    command(
        ids::ACCEPT,
        "Accept",
        "Chooses the focused answer.",
        Dialog,
        &["enter"],
        &["enter"],
    ),
];

/// Names kept for the commands of later alphas, so no command of this one
/// takes them in the palette: the name, and the alpha that brings it.
pub const RESERVED_NAMES: [(&str, &str); 1] = [("hot folder", "alpha.7")];

/// A built-in command by id.
pub fn info(id: &CommandId) -> Option<&'static CommandInfo> {
    CATALOG.iter().find(|info| &info.id == id)
}

/// A command's name, or its id when it is not built in.
pub fn name_of(id: &CommandId) -> String {
    info(id).map_or_else(|| id.to_string(), |info| info.name.to_owned())
}

/// The sort commands and their keys, in the Sort list's order.
pub const SORT_COMMANDS: [(CommandId, SortKey); 11] = [
    (ids::SORT_NAME, SortKey::Name),
    (ids::SORT_EXTENSION, SortKey::Extension),
    (ids::SORT_SIZE, SortKey::Size),
    (ids::SORT_MODIFIED, SortKey::Modified),
    (ids::SORT_CREATED, SortKey::Created),
    (ids::SORT_ACCESSED, SortKey::Accessed),
    (ids::SORT_KIND, SortKey::Kind),
    (ids::SORT_ATTRIBUTES, SortKey::Attributes),
    (ids::SORT_OWNER, SortKey::Owner),
    (ids::SORT_LABEL, SortKey::Label),
    (ids::SORT_UNSORTED, SortKey::Unsorted),
];

/// The sort key a sort command chooses.
pub fn sort_key_for(command: &CommandId) -> Option<SortKey> {
    SORT_COMMANDS
        .iter()
        .find(|(id, _)| id == command)
        .map(|(_, key)| key.clone())
}

/// The command that chooses a sort key.
pub fn sort_command_for(key: &SortKey) -> Option<CommandId> {
    SORT_COMMANDS
        .iter()
        .find(|(_, k)| k == key)
        .map(|(id, _)| id.clone())
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The Sort list (plan section 5.2): every panel sort key, the current
/// one marked "current" with its direction said, the focus on it. Each
/// row's id is its sort command, so Enter runs it, and the same choice
/// again reverses the direction.
pub fn sort_list(current: &SortSpec) -> ListModel {
    let rows = SORT_COMMANDS
        .iter()
        .map(|(id, key)| {
            let row = ListRow::new(id.as_str(), capitalized(&key.word()));
            if *key == current.key {
                let direction = key.direction_words(current.direction);
                if direction.is_empty() {
                    row.with_details(["current"])
                } else {
                    row.with_details(["current".to_owned(), direction.to_owned()])
                }
            } else {
                row
            }
        })
        .collect();
    let mut list = ListModel::new("Sort by").with_rows(rows);
    if let Some(id) = sort_command_for(&current.key) {
        list.focus_id(id.as_str());
    }
    list.set_help("Up and Down choose, Enter sorts, the same key again reverses, Escape closes.");
    list
}

/// Writes a command in the text form the banks and `diro run` share: the
/// id, then each argument as `name=value`. Core's `Display` for
/// [`Command`] is the one definition (the alpha.2 contract step); this
/// stays for the Wave 1 callers.
pub fn command_text(command: &Command) -> String {
    command.to_string()
}

/// Reads a command from its text form (see [`command_text`]), through
/// [`Command::parse_text`]. The error says what is wrong in words, meaning
/// first: "Not a command id: File.Copy".
pub fn parse_command_text(text: &str) -> Result<Command, String> {
    Command::parse_text(text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::{ArgValue, Key, KeyCode, Place, SortDirection};
    use std::collections::HashSet;

    #[test]
    fn every_id_is_valid_and_unique() {
        let mut seen = HashSet::new();
        for info in &CATALOG {
            assert!(CommandId::is_valid(info.id.as_str()), "{}", info.id);
            assert!(seen.insert(info.id.clone()), "duplicate {}", info.id);
            assert!(
                !info.name.is_empty() && info.help.ends_with('.'),
                "{}",
                info.id
            );
        }
    }

    /// Whether text names a key literally, which a rebinding would make
    /// untrue.
    fn names_a_key(text: &str) -> bool {
        let fixed = ["Ctrl+", "Alt+", "Shift+", "Cmd+"];
        let words = ["Escape", "Enter", "Tab", "Backspace", "Insert", "Space"];
        fixed.iter().any(|f| text.contains(f))
            || text.split(|c: char| !c.is_alphanumeric()).any(|w| {
                words.contains(&w)
                    || (w.len() <= 3 && w.starts_with('F') && w[1..].parse::<u8>().is_ok())
            })
    }

    #[test]
    fn no_help_sentence_names_a_key() {
        for info in &CATALOG {
            assert!(!names_a_key(info.help), "{}: {}", info.id, info.help);
        }
        for info in diroxide_config::SCHEMA.iter() {
            assert!(!names_a_key(info.help), "{}: {}", info.key, info.help);
        }
        assert!(names_a_key("Press F5 now"));
        assert!(!names_a_key("Enters the folder"));
    }

    #[test]
    fn reserved_names_are_left_free() {
        for (name, _) in RESERVED_NAMES {
            for info in &CATALOG {
                assert!(!info.name.to_lowercase().contains(name), "{}", info.id);
                assert!(!info.help.to_lowercase().contains(name), "{}", info.id);
            }
        }
    }

    #[test]
    fn default_keys_parse_and_never_use_insert() {
        for info in &CATALOG {
            for keys in [info.terminal, info.window] {
                for text in keys {
                    let key = Key::parse(text).unwrap_or_else(|e| panic!("{}: {e}", info.id));
                    assert!(key.code != KeyCode::Insert, "{}", info.id);
                }
            }
            assert_eq!(info.palette_only, info.terminal.is_empty(), "{}", info.id);
        }
    }

    #[test]
    fn names_fit_a_braille_line_with_their_key() {
        for info in &CATALOG {
            assert!(info.name.chars().count() <= 30, "{}", info.name);
            let row = format!("{}, Ctrl+Shift+F10", info.name);
            assert!(within_cells(&row, info.name), "{row}");
        }
        assert_eq!(name_of(&ids::COPY), "Copy to other panel");
        assert_eq!(name_of(&CommandId::from_static("x.y")), "x.y");
    }

    #[test]
    fn the_phase_0_keys_that_were_decided_away_are_gone() {
        for info in &CATALOG {
            for text in info.terminal.iter().chain(info.window) {
                let key = Key::parse(text).unwrap();
                for gone in ["f10", "ctrl+i", "ctrl+h", "ctrl+shift+s"] {
                    if key == Key::parse(gone).unwrap() {
                        let allowed = (gone == "f10" && info.id == ids::MENU)
                            || (gone == "ctrl+shift+s" && info.id == ids::SORT_LIST);
                        assert!(allowed, "{} still has {gone}", info.id);
                    }
                }
            }
        }
        let quit = info(&ids::QUIT).unwrap();
        assert_eq!(quit.terminal, &["ctrl+q"]);
        assert_eq!(info(&ids::SAY_STATUS).unwrap().terminal, &["ctrl+d"]);
        assert_eq!(info(&ids::DETAILS).unwrap().window, &["alt+enter"]);
    }

    #[test]
    fn the_sort_list_marks_the_current_key_and_its_direction() {
        let list = sort_list(&SortSpec::by(SortKey::Size));
        assert_eq!(list.title(), "Sort by");
        assert_eq!(
            list.announce_focus(),
            "Size, current, largest first, 3 of 11"
        );
        assert_eq!(list.rows()[0].spoken(), "Name");
        let reversed = SortSpec {
            direction: SortDirection::Ascending,
            ..SortSpec::by(SortKey::Size)
        };
        assert!(
            sort_list(&reversed)
                .announce_focus()
                .contains("smallest first")
        );
        assert_eq!(
            sort_list(&SortSpec::by(SortKey::Unsorted)).announce_focus(),
            "The folder's own order, current, 11 of 11"
        );
        for (id, key) in &SORT_COMMANDS {
            assert_eq!(sort_key_for(id).as_ref(), Some(key));
            assert!(info(id).is_some(), "{id}");
        }
        assert_eq!(sort_key_for(&ids::COPY), None);
    }

    #[test]
    fn the_command_text_form_reads_back() {
        let place = Place::parse(if cfg!(windows) {
            r"D:\My Backup"
        } else {
            "/my backup"
        })
        .unwrap();
        let command = Command::new(ids::COPY)
            .with_arg("dest", ArgValue::Place(place.clone()))
            .with_arg("count", ArgValue::Number(3))
            .with_arg("keep_dates", ArgValue::Flag(false))
            .with_arg("note", ArgValue::Text("say \"hi\"".into()))
            .with_arg("plain", ArgValue::Text("12".into()));
        let text = command_text(&command);
        assert_eq!(parse_command_text(&text).unwrap(), command, "{text}");
        assert!(text.starts_with("file.copy count=3 dest=\""), "{text}");
        assert!(text.contains(" plain=\"12\""), "{text}");
        for info in &CATALOG {
            let c = Command::new(info.id.clone());
            assert_eq!(parse_command_text(&command_text(&c)).unwrap(), c);
        }
        assert_eq!(
            parse_command_text("File.Copy").unwrap_err(),
            "Not a command id: File.Copy"
        );
        assert_eq!(
            parse_command_text("file.copy dest").unwrap_err(),
            "No value for dest: write dest=value"
        );
        assert_eq!(
            parse_command_text("file.copy note=\"open").unwrap_err(),
            "a quotation mark is not closed"
        );
        assert!(parse_command_text("   ").is_err());
    }
}
