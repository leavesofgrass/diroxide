//! The viewer in the terminal: its rows drawn through the theme, the
//! token classes as colors with an attribute where a class has a natural
//! one (bold keywords and headings, underlined links, a reversed
//! problem), and plain in screen reader mode, where highlighting is
//! heard through the viewer's commands instead (R17).
//!
//! The rules (plan section 5.10, R17): color never carries meaning alone,
//! since every class is also said in words by Say Token Kind and Say Line
//! by Kind; `NO_COLOR`, a terminal without color and one with only the
//! sixteen base colors draw attributes alone; the `highlighting` setting
//! chooses (automatic: colored except in screen reader mode; on; off);
//! the text never changes with the styles, so a colored frame and a plain
//! one read the same; the hardware cursor sits at the focused row's first
//! cell. Search matches are marked with the theme's search-match role,
//! and in sighted mode the token at the point is reversed.
//!
//! [`body`] builds the rows for a region the screen gives the viewer (W2f
//! places it); [`render`] draws a whole frame of its own, the title, the
//! rows and the status line, for tests and as the reference layout.

use std::collections::BTreeMap;
use std::path::Path;

use diroxide_app::viewer::{ClassRun, HighlightSetting, TokenClass, Viewer};
use diroxide_theme::{
    Attrs, ColorSupport, PanelRole, Registry, Rgb, TermColor, role_style, term_color,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::style::Styles;
use crate::view::{cells, clean, line_text};

/// How the viewer is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerLook {
    /// The columns the frame has.
    pub width: u16,
    /// The rows the frame has.
    pub height: u16,
    /// Screen reader mode: plain rows, unless the setting says on.
    pub screen_reader: bool,
    /// The terminal's color level.
    pub color: ColorSupport,
    /// The `highlighting` setting.
    pub highlighting: HighlightSetting,
}

impl ViewerLook {
    /// Whether code is drawn with its classes' styles.
    pub fn colors_code(&self) -> bool {
        match self.highlighting {
            HighlightSetting::On => true,
            HighlightSetting::Off => false,
            HighlightSetting::Auto => !self.screen_reader,
        }
    }
}

/// The styles of the token classes in one theme at one color level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeStyles {
    classes: Vec<(TokenClass, Style)>,
}

/// The theme's token-class role of each class (P2b, W2d's request 1), so
/// `[colors]` can override a class and the contrast check covers it; a
/// plain run has none. The roles keep the attributes the classes have by
/// nature (bold keywords and headings, underlined links, a reversed
/// problem); italic is never used, since many terminals draw it poorly
/// (R17).
pub fn class_role(class: TokenClass) -> Option<PanelRole> {
    use TokenClass as C;
    Some(match class {
        C::Plain => return None,
        C::Keyword => PanelRole::TokenKeyword,
        C::String => PanelRole::TokenString,
        C::Comment => PanelRole::TokenComment,
        C::Number => PanelRole::TokenNumber,
        C::Constant => PanelRole::TokenConstant,
        C::FunctionName => PanelRole::TokenFunction,
        C::TypeName => PanelRole::TokenType,
        C::Variable => PanelRole::TokenVariable,
        C::Operator => PanelRole::TokenOperator,
        C::Punctuation => PanelRole::TokenPunctuation,
        C::Heading => PanelRole::TokenHeading,
        C::Link => PanelRole::TokenLink,
        C::Emphasis => PanelRole::TokenEmphasis,
        C::Code => PanelRole::TokenCode,
        C::Tag => PanelRole::TokenTag,
        C::Attribute => PanelRole::TokenAttribute,
        C::Problem => PanelRole::TokenProblem,
        // A class a later preview crate adds draws plain until it has a role.
        _ => return None,
    })
}

fn modifiers(attrs: Attrs) -> Modifier {
    let mut m = Modifier::empty();
    if attrs.bold {
        m |= Modifier::BOLD;
    }
    if attrs.underline {
        m |= Modifier::UNDERLINED;
    }
    if attrs.reverse {
        m |= Modifier::REVERSED;
    }
    m
}

fn color(rgb: Rgb, support: ColorSupport) -> Option<Color> {
    match term_color(rgb, support)? {
        TermColor::Rgb(c) => Some(Color::Rgb(c.r, c.g, c.b)),
        TermColor::Indexed(i) => Some(Color::Indexed(i)),
    }
}

impl CodeStyles {
    /// The classes' styles of the theme named `theme` (a built-in one, or a
    /// file in `themes_dir`) at a color level: colors and attributes at
    /// 256 colors and true color, attributes alone below. The same as
    /// [`CodeStyles::with_colors`] with no `[colors]` overrides.
    pub fn new(theme: &str, themes_dir: Option<&Path>, support: ColorSupport) -> Self {
        Self::with_colors(theme, &BTreeMap::new(), themes_dir, support)
    }

    /// The classes' styles through the theme's token-class roles
    /// ([`class_role`]), with the owner's `[colors]` overrides by role key
    /// (`token_keyword = "#ffaa00"`), which replace a class's text color
    /// and keep its attribute. Only the text color is set, so a run keeps
    /// its row's background; a class the theme draws as plain text
    /// (variables, operators, punctuation) keeps the row's own color unless
    /// the owner colors it.
    pub fn with_colors(
        theme: &str,
        colors: &BTreeMap<String, String>,
        themes_dir: Option<&Path>,
        support: ColorSupport,
    ) -> Self {
        let mut registry = Registry::builtin();
        if let Some(dir) = themes_dir {
            let _ = registry.load_dir(dir);
        }
        let (chosen, _) = registry.resolve(theme);
        let text = role_style(chosen, PanelRole::Text);
        let classes = TokenClass::ALL
            .into_iter()
            .map(|class| {
                let Some(role) = class_role(class) else {
                    return (class, Style::default());
                };
                let theirs = role_style(chosen, role);
                let owners = colors.get(role.key()).and_then(|c| Rgb::parse(c).ok());
                let mut style = Style::default().add_modifier(modifiers(theirs.attributes));
                let plain =
                    theirs.foreground == text.foreground && theirs.attributes == Attrs::NONE;
                let foreground = owners.or((!plain).then_some(theirs.foreground));
                if let Some(fg) = foreground.and_then(|rgb| color(rgb, support)) {
                    style = style.fg(fg);
                }
                (class, style)
            })
            .collect();
        Self { classes }
    }

    /// No styles at all: screen reader mode, or the setting off.
    pub fn plain() -> Self {
        Self {
            classes: TokenClass::ALL
                .into_iter()
                .map(|class| (class, Style::default()))
                .collect(),
        }
    }

    /// A class's style.
    pub fn get(&self, class: TokenClass) -> Style {
        self.classes
            .iter()
            .find(|(c, _)| *c == class)
            .map_or_else(Style::default, |(_, s)| *s)
    }
}

/// What a span of a row is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    /// A role of the screen: the row's text, the focused row, the title.
    Role(PanelRole),
    /// A token class, over the row's role.
    Class(TokenClass),
}

/// A span of a line: its first cell, its text and how it is drawn, with
/// the marks laid over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// The first cell.
    pub x: u16,
    /// The text, hidden characters already shown as question marks.
    pub text: String,
    /// How it is drawn.
    pub paint: Paint,
    /// Inside a search match.
    pub matched: bool,
    /// The token at the point (sighted mode).
    pub at_point: bool,
}

/// A line of the frame: its role (the background it fills) and its spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The role that fills the line.
    pub role: PanelRole,
    /// Whether the role fills the whole width (the focused row's bar).
    pub fill: bool,
    /// The spans, left to right.
    pub spans: Vec<Span>,
}

/// The viewer's rows for a region of `width` by `height`: one line a
/// row, the focused one filled with the focused-row role, and where the
/// hardware cursor goes, relative to the region (the focused row's first
/// cell).
pub fn body(
    viewer: &mut Viewer,
    width: u16,
    height: u16,
    look: &ViewerLook,
) -> (Vec<Line>, (u16, u16)) {
    viewer.set_size(usize::from(width.max(1)), usize::from(height.max(1)));
    let rows = viewer.rows();
    let colored = look.colors_code();
    let point = if look.screen_reader {
        None
    } else {
        viewer.point_token()
    };
    let mut lines = Vec::with_capacity(rows.len());
    let mut cursor = (0, 0);
    for (y, row) in rows.iter().enumerate() {
        let role = if row.focused {
            cursor = (0, y as u16);
            PanelRole::FocusedRow
        } else {
            PanelRole::Text
        };
        let runs = if colored {
            viewer.row_runs(row)
        } else {
            Vec::new()
        };
        let matches = if look.screen_reader {
            Vec::new()
        } else {
            viewer.match_ranges(&row.text)
        };
        let point_here = point.clone().filter(|_| row.focused).map(|range| {
            let shift = |b: usize| (b + row.prefix).saturating_sub(row.from);
            shift(range.start)..shift(range.end)
        });
        let spans = spans_of(&row.text, &runs, &matches, point_here, role);
        lines.push(Line {
            role,
            fill: row.focused,
            spans,
        });
    }
    (lines, cursor)
}

/// Cuts a row's text at every run, match and point boundary into spans.
fn spans_of(
    text: &str,
    runs: &[ClassRun],
    matches: &[std::ops::Range<usize>],
    point: Option<std::ops::Range<usize>>,
    role: PanelRole,
) -> Vec<Span> {
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for run in runs {
        cuts.extend([run.start as usize, run.end as usize]);
    }
    for m in matches {
        cuts.extend([m.start, m.end]);
    }
    if let Some(p) = &point {
        cuts.extend([p.start, p.end]);
    }
    cuts.retain(|c| *c <= text.len() && text.is_char_boundary(*c));
    cuts.sort_unstable();
    cuts.dedup();
    let mut spans = Vec::new();
    let mut x = 0usize;
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end {
            continue;
        }
        let piece = clean(&text[start..end]);
        let class = runs
            .iter()
            .find(|r| (r.start as usize) <= start && start < r.end as usize)
            .map(|r| r.class);
        let paint = match class {
            Some(class) if role != PanelRole::FocusedRow || class != TokenClass::Plain => {
                Paint::Class(class)
            }
            _ => Paint::Role(role),
        };
        let matched = matches.iter().any(|m| m.start <= start && start < m.end);
        let at_point = point
            .as_ref()
            .is_some_and(|p| p.start <= start && start < p.end);
        let width = cells(&piece);
        spans.push(Span {
            x: x.min(usize::from(u16::MAX)) as u16,
            text: piece,
            paint,
            matched,
            at_point,
        });
        x += width;
    }
    spans
}

/// Paints lines into a buffer area with the screen's styles and the code
/// styles.
pub fn paint(lines: &[Line], area: Rect, styles: &Styles, code: &CodeStyles, buffer: &mut Buffer) {
    for (y, line) in lines.iter().enumerate() {
        let y = area.y + y as u16;
        if y >= area.bottom() {
            break;
        }
        let base = styles.get(line.role);
        if line.fill {
            buffer.set_style(Rect::new(area.x, y, area.width, 1), base);
        }
        for span in &line.spans {
            let x = area.x + span.x;
            if x >= area.right() {
                continue;
            }
            let mut style = match span.paint {
                Paint::Role(role) => styles.get(role),
                Paint::Class(class) => base.patch(code.get(class)),
            };
            if span.matched {
                style = style.patch(styles.get(PanelRole::SearchMatch));
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if span.at_point {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let room = usize::from(area.right() - x);
            buffer.set_stringn(x, y, &span.text, room, style);
        }
    }
}

/// A whole frame as text and its cursor: the title, the rows, the status
/// line; the styles' buffer beside it for tests that read the colors.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The lines as a reader gets them, trailing spaces trimmed.
    pub lines: Vec<String>,
    /// The hardware cursor: column, line.
    pub cursor: (u16, u16),
    /// The painted buffer.
    pub buffer: Buffer,
}

/// Draws the viewer as a frame of its own: the title on the first line,
/// the rows, and the status line on the last.
pub fn render(viewer: &mut Viewer, look: &ViewerLook, styles: &Styles, code: &CodeStyles) -> Frame {
    let width = look.width.max(20);
    let height = look.height.max(3);
    let rows_h = height - 2;
    let (rows, cursor) = body(viewer, width, rows_h, look);
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    buffer.set_style(buffer.area, styles.get(PanelRole::Page));
    let title = clean(viewer.title());
    buffer.set_stringn(
        0,
        0,
        &title,
        usize::from(width),
        styles.get(PanelRole::ActiveTitle),
    );
    paint(
        &rows,
        Rect::new(0, 1, width, rows_h),
        styles,
        code,
        &mut buffer,
    );
    let status = clean(&viewer.status());
    let status_style = styles.get(PanelRole::StatusLine);
    buffer.set_style(Rect::new(0, height - 1, width, 1), status_style);
    buffer.set_stringn(0, height - 1, &status, usize::from(width), status_style);
    Frame {
        lines: (0..height).map(|y| line_text(&buffer, y)).collect(),
        cursor: (cursor.0, cursor.1 + 1),
        buffer,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use diroxide_app::Runtime;
    use diroxide_app::viewer::{ViewerOptions, highlight::Session};
    use diroxide_core::Place;

    use super::*;

    fn sample(name: &str) -> Viewer {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/viewer/code")
            .join(name);
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let text = path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned();
        let place = Place::parse(&text).unwrap();
        let runtime = Runtime::with_local();
        let (mut viewer, _) =
            Viewer::open(runtime.registry(), &place, None, ViewerOptions::default()).unwrap();
        viewer.set_highlighter(Session::new());
        viewer.wait_highlight();
        viewer
    }

    fn look(screen_reader: bool, color: ColorSupport) -> ViewerLook {
        ViewerLook {
            width: 60,
            height: 14,
            screen_reader,
            color,
            highlighting: HighlightSetting::Auto,
        }
    }

    fn draw(viewer: &mut Viewer, look: ViewerLook) -> Frame {
        let styles = Styles::new("galaxy", &BTreeMap::new(), None, look.color);
        let code = if look.colors_code() {
            CodeStyles::new("galaxy", None, look.color)
        } else {
            CodeStyles::plain()
        };
        render(viewer, &look, &styles, &code)
    }

    /// The cells of one line of a frame.
    fn row(frame: &Frame, y: usize) -> Vec<ratatui::buffer::Cell> {
        (0..frame.buffer.area.width)
            .map(|x| frame.buffer[(x, y as u16)].clone())
            .collect()
    }

    #[test]
    fn a_colored_frame_and_a_plain_one_read_the_same() {
        let mut viewer = sample("main.rs");
        let colored = draw(&mut viewer, look(false, ColorSupport::TrueColor));
        let plain = draw(&mut viewer, look(true, ColorSupport::TrueColor));
        let no_color = draw(&mut viewer, look(false, ColorSupport::NoColor));
        assert_eq!(colored.lines, plain.lines, "color never changes the text");
        assert_eq!(colored.lines, no_color.lines);
        assert!(colored.lines[0].starts_with("main.rs, in "));
        assert_eq!(colored.lines[13], "Line 1 of 55, 1 percent");
        let y = colored
            .lines
            .iter()
            .position(|l| l.starts_with("pub struct Config"))
            .unwrap();
        // "pub" is a keyword, "Config" a type name, the space between plain.
        let sighted = row(&colored, y);
        assert_ne!(sighted[0].fg, sighted[3].fg, "a keyword is colored");
        assert!(sighted[0].modifier.contains(Modifier::BOLD), "and bold");
        assert_ne!(sighted[0].fg, sighted[11].fg, "a type name has its own");
        // Screen reader mode draws the rows plain: one color, no class
        // attributes.
        let heard = row(&plain, y);
        assert_eq!(heard[0].fg, heard[11].fg);
        assert!(heard.iter().all(|c| c.modifier.is_empty()));
        // Without color the keyword keeps its bold, and nothing has a color.
        let attrs = row(&no_color, y);
        assert!(attrs[0].modifier.contains(Modifier::BOLD));
        assert!(attrs.iter().all(|c| c.fg == Color::Reset));
    }

    #[test]
    fn the_cursor_sits_on_the_focused_rows_first_cell() {
        let mut viewer = sample("main.rs");
        viewer.jump("4");
        let frame = draw(&mut viewer, look(true, ColorSupport::NoColor));
        assert_eq!(frame.cursor.0, 0);
        assert_eq!(frame.lines[usize::from(frame.cursor.1)], "use std::env;");
        let focused = frame.buffer[(0, frame.cursor.1)].modifier;
        assert!(
            focused.contains(Modifier::REVERSED),
            "the focus has an attribute"
        );
    }

    /// W2d's request 1 (P2b): classes map onto the theme's token-class
    /// roles, and `[colors]` overrides a class by its role's key.
    #[test]
    fn classes_take_their_roles_and_the_owners_colors() {
        for class in TokenClass::ALL {
            match class_role(class) {
                Some(role) => assert!(role.is_token(), "{}", class.word()),
                None => assert_eq!(class, TokenClass::Plain),
            }
        }
        let theme = Registry::builtin();
        let (galaxy, _) = theme.resolve("galaxy");
        let truecolor = CodeStyles::new("galaxy", None, ColorSupport::TrueColor);
        let keyword = role_style(galaxy, PanelRole::TokenKeyword).foreground;
        assert_eq!(
            truecolor.get(TokenClass::Keyword).fg,
            Some(Color::Rgb(keyword.r, keyword.g, keyword.b))
        );
        assert_eq!(
            truecolor.get(TokenClass::Variable).fg,
            None,
            "plain by default"
        );
        let colors: BTreeMap<String, String> = [
            ("token_keyword".to_owned(), "#ffaa00".to_owned()),
            ("token_variable".to_owned(), "#00aaff".to_owned()),
        ]
        .into_iter()
        .collect();
        let owners = CodeStyles::with_colors("galaxy", &colors, None, ColorSupport::TrueColor);
        assert_eq!(
            owners.get(TokenClass::Keyword).fg,
            Some(Color::Rgb(0xff, 0xaa, 0))
        );
        assert!(
            owners
                .get(TokenClass::Keyword)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(
            owners.get(TokenClass::Variable).fg,
            Some(Color::Rgb(0, 0xaa, 0xff))
        );
        let none = CodeStyles::with_colors("galaxy", &colors, None, ColorSupport::NoColor);
        assert_eq!(
            none.get(TokenClass::Keyword).fg,
            None,
            "no color, no override"
        );
    }

    #[test]
    fn every_colored_class_keeps_its_attribute_without_color() {
        let none = CodeStyles::new("galaxy", None, ColorSupport::NoColor);
        let ansi16 = CodeStyles::new("galaxy", None, ColorSupport::Ansi16);
        for class in TokenClass::ALL {
            assert_eq!(none.get(class).fg, None, "{}", class.word());
            assert_eq!(none.get(class), ansi16.get(class));
            assert!(!none.get(class).add_modifier.contains(Modifier::ITALIC));
        }
        for class in [
            TokenClass::Keyword,
            TokenClass::Heading,
            TokenClass::Emphasis,
        ] {
            assert!(none.get(class).add_modifier.contains(Modifier::BOLD));
        }
        let link = none.get(TokenClass::Link).add_modifier;
        assert!(link.contains(Modifier::UNDERLINED));
        let problem = none.get(TokenClass::Problem).add_modifier;
        assert!(problem.contains(Modifier::REVERSED));
        let deep = CodeStyles::new("galaxy", None, ColorSupport::Ansi256);
        assert!(matches!(
            deep.get(TokenClass::String).fg,
            Some(Color::Indexed(_))
        ));
        let truecolor = CodeStyles::new("galaxy", None, ColorSupport::TrueColor);
        assert!(matches!(
            truecolor.get(TokenClass::String).fg,
            Some(Color::Rgb(..))
        ));
        assert_eq!(
            CodeStyles::plain().get(TokenClass::Keyword),
            Style::default()
        );
    }

    #[test]
    fn the_setting_decides_whether_code_is_colored() {
        let mut look = look(true, ColorSupport::TrueColor);
        assert!(!look.colors_code(), "automatic: plain for a screen reader");
        look.highlighting = HighlightSetting::On;
        assert!(look.colors_code());
        look.highlighting = HighlightSetting::Off;
        look.screen_reader = false;
        assert!(!look.colors_code());
        look.highlighting = HighlightSetting::Auto;
        assert!(look.colors_code());
    }

    #[test]
    fn search_matches_and_the_point_are_marked_with_attributes_in_sighted_mode() {
        let mut viewer = sample("main.rs");
        viewer.search("Config", false);
        viewer.home();
        let frame = draw(&mut viewer, look(false, ColorSupport::NoColor));
        let y = frame
            .lines
            .iter()
            .position(|l| l.starts_with("pub struct Config"))
            .unwrap();
        let cells = row(&frame, y);
        assert!(
            cells[11].modifier.contains(Modifier::UNDERLINED),
            "the match"
        );
        assert!(!cells[0].modifier.contains(Modifier::UNDERLINED));
        viewer.jump("21");
        viewer.right();
        let frame = draw(&mut viewer, look(false, ColorSupport::NoColor));
        let line = usize::from(frame.cursor.1);
        let focused = row(&frame, line);
        // The focused row is reversed; the token at the point is reversed
        // again, so it stands out of the bar.
        let start = frame.lines[line].find("fn").unwrap();
        assert_ne!(focused[start].modifier, focused[0].modifier);
        let heard = draw(&mut viewer, look(true, ColorSupport::NoColor));
        assert_eq!(frame.lines, heard.lines);
    }

    #[test]
    fn hex_rows_draw_plain_inside_forty_cells() {
        let mut viewer = sample("main.rs");
        viewer.toggle_hex();
        let mut narrow = look(true, ColorSupport::TrueColor);
        narrow.width = 40;
        let frame = draw(&mut viewer, narrow);
        assert!(
            frame.lines[1].starts_with("000000: 2f 2f 21 20"),
            "{:?}",
            frame.lines
        );
        assert!(frame.lines.iter().all(|l| l.chars().count() <= 40));
        let second = row(&frame, 2);
        assert!(
            second.iter().all(|c| c.fg == second[0].fg),
            "one color a row"
        );
    }
}
