// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The resolved theme model: semantic roles, text attributes, highlight
//! styles, and [`Theme`].

use std::fmt;

use toml::Table;

use crate::color::Rgb;

/// Whether a theme is light, dark, or high contrast. High-contrast themes are
/// held to WCAG AAA (7:1) for text instead of AA (4.5:1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    /// Dark text on a light background.
    Light,
    /// Light text on a dark background.
    Dark,
    /// Maximum contrast; text at 7:1 or more.
    HighContrast,
}

impl ThemeKind {
    /// The key used in theme files: `light`, `dark`, or `high-contrast`.
    pub fn key(self) -> &'static str {
        match self {
            ThemeKind::Light => "light",
            ThemeKind::Dark => "dark",
            ThemeKind::HighContrast => "high-contrast",
        }
    }

    /// Reads a kind key; accepts `high_contrast` and `contrast` as synonyms.
    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Some(ThemeKind::Light),
            "dark" => Some(ThemeKind::Dark),
            "high-contrast" | "high_contrast" | "highcontrast" | "contrast" => {
                Some(ThemeKind::HighContrast)
            }
            _ => None,
        }
    }

    /// The label read aloud.
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Light => "light",
            ThemeKind::Dark => "dark",
            ThemeKind::HighContrast => "high contrast",
        }
    }
}

macro_rules! roles {
    (
        $(#[$m:meta])* $vis:vis enum $name:ident {
            $( $(#[$vm:meta])* $v:ident = ($key:literal, $label:literal), )+
        }
    ) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $( $(#[$vm])* $v, )+
        }

        impl $name {
            /// Every role, in file order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];
            /// The number of roles.
            pub const COUNT: usize = [$($name::$v),+].len();

            /// The key used in theme files, `snake_case`.
            pub fn key(self) -> &'static str {
                match self { $($name::$v => $key,)+ }
            }

            /// The name read aloud, lowercase words.
            pub fn label(self) -> &'static str {
                match self { $($name::$v => $label,)+ }
            }

            /// Reads a file key.
            pub fn from_key(key: &str) -> Option<Self> {
                match key { $($key => Some($name::$v),)+ _ => None }
            }

            /// Position in [`Self::ALL`].
            pub fn index(self) -> usize {
                self as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.label())
            }
        }
    };
}

roles! {
    /// A single color in the `[colors]` table. Most are text colors drawn on
    /// the background; `Background`, `Surface`, and `CodeBackground` are
    /// backgrounds.
    pub enum ColorRole {
        /// The page.
        Background = ("background", "background"),
        /// Panels, lists, and the minibuffer; holds body text only.
        Surface = ("surface", "panel background"),
        /// Body text.
        Text = ("text", "text"),
        /// Secondary text on the page (not on panels): hints, line numbers,
        /// rules and borders.
        DimText = ("dim_text", "dim text"),
        /// Level 1 headings.
        Heading1 = ("heading1", "heading 1"),
        /// Level 2 headings.
        Heading2 = ("heading2", "heading 2"),
        /// Level 3 headings.
        Heading3 = ("heading3", "heading 3"),
        /// Level 4 headings.
        Heading4 = ("heading4", "heading 4"),
        /// Level 5 headings.
        Heading5 = ("heading5", "heading 5"),
        /// Level 6 headings.
        Heading6 = ("heading6", "heading 6"),
        /// Links (always underlined as well).
        Link = ("link", "link"),
        /// Inline code and code blocks.
        Code = ("code", "code"),
        /// Behind inline code and code blocks.
        CodeBackground = ("code_background", "code background"),
        /// Block quotes (always italic in the terminal as well).
        Quote = ("quote", "quote"),
        /// Error messages (always bold as well, and always worded as errors).
        Error = ("error", "error"),
        /// Buttons, fields, and the selected row: a step up from the panel.
        /// Derived from the surface unless the file gives it.
        Raised = ("raised", "raised surface"),
        /// Panel edges and dividers; decorative, never the only cue.
        Border = ("border", "border"),
        /// Edges of buttons, fields, and switches, at 3 to 1.
        ControlBorder = ("control_border", "control border"),
        /// Muted text on panels, held to the text floor on the surface.
        PanelDimText = ("panel_dim_text", "panel dim text"),
        /// The primary button, a switch that is on, the focused row,
        /// progress, and the RSVP pivot.
        Accent = ("accent", "accent"),
        /// Text drawn on the accent.
        OnAccent = ("on_accent", "text on accent"),
        /// Text of controls that are unavailable, at 3 to 1.
        Disabled = ("disabled", "disabled text"),
        /// The text caret.
        Caret = ("caret", "caret"),
        /// The thin line between a control and its focus ring.
        FocusInner = ("focus_inner", "inner focus line"),
        /// The band behind the reading line.
        Ruler = ("ruler", "reading ruler"),
        /// The rows around the reading line; a faint band.
        RulerBand = ("ruler_band", "ruler band"),
        /// The thick underline under a difficult word.
        DifficultWord = ("difficult_word", "difficult word mark"),
        /// The dot between syllables.
        SyllableMark = ("syllable_mark", "syllable mark"),
        /// The dotted underline under a misspelled word.
        Misspelling = ("misspelling", "misspelling mark"),
        /// The double underline under a writing suggestion.
        Lint = ("lint", "writing suggestion mark"),
    }
}

/// What a color is for, which decides how it is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoleClass {
    /// A fill that text sits on; each text color drawn on it meets the
    /// text floor there.
    Surface,
    /// Text: 4.5 to 1 on each surface it is drawn on, 7 to 1 in
    /// high-contrast themes (disabled text: 3 to 1).
    Text,
    /// A line, ring, bar, or dot that carries a state: 3 to 1 against the
    /// colors beside it.
    Indicator,
    /// A tint behind text marking a range: text on it meets the floor. A
    /// band is never the only cue.
    Band,
    /// A decorative line, such as a divider: no ratio, never the only cue.
    Decorative,
}

impl RoleClass {
    /// The class's name read aloud.
    pub fn label(self) -> &'static str {
        match self {
            RoleClass::Surface => "surface",
            RoleClass::Text => "text",
            RoleClass::Indicator => "indicator",
            RoleClass::Band => "band",
            RoleClass::Decorative => "decorative",
        }
    }
}

roles! {
    /// A foreground, background, and attribute style in the `[styles]` table:
    /// highlights drawn over text, the status bar, and focus.
    pub enum StyleRole {
        /// Selected text.
        Selection = ("selection", "selection"),
        /// The word being spoken.
        SpokenWord = ("spoken_word", "spoken word"),
        /// The band behind the sentence being spoken.
        SpokenSentence = ("spoken_sentence", "spoken sentence"),
        /// A search match.
        FindHit = ("find_hit", "find match"),
        /// The search match at the cursor.
        CurrentFindHit = ("current_find_hit", "current find match"),
        /// A bookmarked word.
        Bookmark = ("bookmark", "bookmark"),
        /// Text with a note.
        Note = ("note", "note"),
        /// The status and title bars.
        StatusBar = ("status_bar", "status bar"),
        /// The focused item in a list, and the focus ring color.
        Focus = ("focus", "focus"),
    }
}

impl ColorRole {
    /// The fifteen roles worked out at load from the theme's own colors
    /// when the file does not give them, in file order.
    pub const DERIVED: &'static [ColorRole] = &[
        ColorRole::Raised,
        ColorRole::Border,
        ColorRole::ControlBorder,
        ColorRole::PanelDimText,
        ColorRole::Accent,
        ColorRole::OnAccent,
        ColorRole::Disabled,
        ColorRole::Caret,
        ColorRole::FocusInner,
        ColorRole::Ruler,
        ColorRole::RulerBand,
        ColorRole::DifficultWord,
        ColorRole::SyllableMark,
        ColorRole::Misspelling,
        ColorRole::Lint,
    ];

    /// True for the roles in [`Self::DERIVED`]: derived at load unless a
    /// `[colors]` key of the same name gives them, and never copied from a
    /// base theme's derived value through `inherits`.
    pub fn is_derived(self) -> bool {
        self >= ColorRole::Raised
    }

    /// The role's class, which decides its check.
    pub fn class(self) -> RoleClass {
        use ColorRole as C;
        match self {
            C::Background | C::Surface | C::CodeBackground | C::Raised => RoleClass::Surface,
            C::Border => RoleClass::Decorative,
            C::ControlBorder
            | C::Accent
            | C::Caret
            | C::FocusInner
            | C::DifficultWord
            | C::SyllableMark
            | C::Misspelling
            | C::Lint => RoleClass::Indicator,
            C::Ruler | C::RulerBand => RoleClass::Band,
            _ => RoleClass::Text,
        }
    }

    /// True for colors drawn as text on the page: the original text roles
    /// (everything before [`Self::DERIVED`] except the three backgrounds).
    /// The derived text roles sit on panels, controls, or the accent.
    pub fn is_text(self) -> bool {
        !self.is_derived() && self.class() == RoleClass::Text
    }

    /// The heading role for level 1 to 6.
    pub fn heading(level: u8) -> Option<Self> {
        Some(match level {
            1 => ColorRole::Heading1,
            2 => ColorRole::Heading2,
            3 => ColorRole::Heading3,
            4 => ColorRole::Heading4,
            5 => ColorRole::Heading5,
            6 => ColorRole::Heading6,
            _ => return None,
        })
    }

    /// Attributes every renderer adds to this role so it is never marked by
    /// color alone: headings bold, links underlined, quotes italic, errors
    /// bold. Fixed, not configurable. Dim text gets no dim attribute, because
    /// terminals dim by unpredictable amounts.
    pub fn attributes(self) -> Attrs {
        match self {
            ColorRole::Heading1
            | ColorRole::Heading2
            | ColorRole::Heading3
            | ColorRole::Heading4
            | ColorRole::Heading5
            | ColorRole::Heading6
            | ColorRole::Error => Attrs::BOLD,
            ColorRole::Link => Attrs::UNDERLINE,
            ColorRole::Quote => Attrs::ITALIC,
            _ => Attrs::NONE,
        }
    }
}

impl StyleRole {
    /// True for highlights drawn over document text, which must carry a
    /// non-color cue (everything except the status bar).
    pub fn is_highlight(self) -> bool {
        self != StyleRole::StatusBar
    }

    /// Attributes used when color is off (`NO_COLOR`, monochrome terminals)
    /// or when a theme gives a highlight no attributes. Chosen so that the
    /// highlights that appear together differ: the spoken word from its
    /// sentence, the current match from the other matches.
    pub fn no_color_attributes(self) -> Attrs {
        match self {
            StyleRole::Selection | StyleRole::StatusBar => Attrs::REVERSE,
            StyleRole::SpokenWord | StyleRole::Focus => Attrs::REVERSE.with(Attrs::BOLD),
            StyleRole::SpokenSentence | StyleRole::FindHit => Attrs::UNDERLINE,
            StyleRole::CurrentFindHit => Attrs::REVERSE.with(Attrs::BOLD).with(Attrs::UNDERLINE),
            StyleRole::Bookmark => Attrs::BOLD.with(Attrs::UNDERLINE),
            StyleRole::Note => Attrs::ITALIC.with(Attrs::UNDERLINE),
        }
    }
}

/// Attributes used for user highlights when color is off.
pub const USER_HIGHLIGHT_NO_COLOR: Attrs = Attrs::BOLD;

/// Text attributes. Every highlight carries at least one, so no state is
/// shown by color alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs {
    /// Bold (in terminals, may also brighten the color).
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Reverse video: foreground and background swap.
    pub reverse: bool,
}

impl Attrs {
    /// No attributes.
    pub const NONE: Attrs = Attrs {
        bold: false,
        italic: false,
        underline: false,
        reverse: false,
    };
    /// Bold only.
    pub const BOLD: Attrs = Attrs {
        bold: true,
        ..Attrs::NONE
    };
    /// Italic only.
    pub const ITALIC: Attrs = Attrs {
        italic: true,
        ..Attrs::NONE
    };
    /// Underline only.
    pub const UNDERLINE: Attrs = Attrs {
        underline: true,
        ..Attrs::NONE
    };
    /// Reverse video only.
    pub const REVERSE: Attrs = Attrs {
        reverse: true,
        ..Attrs::NONE
    };

    /// Both sets of attributes.
    pub const fn with(self, o: Attrs) -> Attrs {
        Attrs {
            bold: self.bold || o.bold,
            italic: self.italic || o.italic,
            underline: self.underline || o.underline,
            reverse: self.reverse || o.reverse,
        }
    }

    /// True when no attribute is set.
    pub fn is_empty(self) -> bool {
        self == Attrs::NONE
    }

    /// The attribute names in file order: `bold`, `italic`, `underline`,
    /// `reverse`.
    pub fn names(self) -> Vec<&'static str> {
        let mut v = Vec::new();
        for (on, name) in [
            (self.bold, "bold"),
            (self.italic, "italic"),
            (self.underline, "underline"),
            (self.reverse, "reverse"),
        ] {
            if on {
                v.push(name);
            }
        }
        v
    }

    /// One attribute by name.
    pub fn from_name(name: &str) -> Option<Attrs> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "bold" => Attrs::BOLD,
            "italic" => Attrs::ITALIC,
            "underline" | "underlined" => Attrs::UNDERLINE,
            "reverse" | "reversed" => Attrs::REVERSE,
            _ => return None,
        })
    }
}

/// A highlight style as stored in a theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// Text color inside the highlight.
    pub foreground: Rgb,
    /// The band behind the text, or `None` for no band (the page shows
    /// through).
    pub background: Option<Rgb>,
    /// Attributes; highlights need at least one.
    pub attributes: Attrs,
    /// Keys this version does not know, kept for round trips.
    pub extra: Table,
}

impl Style {
    /// A style with no unknown keys.
    pub fn new(foreground: Rgb, background: Option<Rgb>, attributes: Attrs) -> Self {
        Style {
            foreground,
            background,
            attributes,
            extra: Table::new(),
        }
    }
}

/// A style with reverse video applied and the page filled in: the colors
/// that actually appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The text color shown.
    pub foreground: Rgb,
    /// The band color shown (the page when the style has no band).
    pub background: Rgb,
    /// Attributes other than reverse video (already applied to the colors).
    pub attributes: Attrs,
    /// False when the page shows through (no band).
    pub has_band: bool,
}

/// One of the colors a reader can highlight text with.
#[derive(Clone, Debug, PartialEq)]
pub struct UserHighlight {
    /// The color's name, read aloud and used in highlight lists (`yellow`).
    pub name: String,
    /// How it looks.
    pub style: Style,
}

/// Descriptive fields in a theme's `[theme]` table.
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    /// Name used in settings and on the command line (`galaxy`).
    pub name: String,
    /// Name read aloud and shown in lists (`Galaxy`).
    pub display_name: String,
    /// Light, dark, or high contrast.
    pub kind: ThemeKind,
    /// One sentence about the theme.
    pub description: String,
    /// Who made it.
    pub author: Option<String>,
    /// Where it came from: `star` for star's palettes, `user` for the
    /// themes folder.
    pub origin: String,
    /// The light or dark partner used when following the system setting.
    pub counterpart: Option<String>,
    /// The theme this one was based on, if any.
    pub inherits: Option<String>,
    /// Keys this version does not know, kept for round trips.
    pub extra: Table,
}

impl Meta {
    /// The theme's `tags` (for example `soft`), kept in the file's
    /// `[theme]` table; empty when there are none.
    pub fn tags(&self) -> Vec<&str> {
        match self.extra.get("tags") {
            Some(toml::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect(),
            Some(toml::Value::String(s)) => vec![s.as_str()],
            _ => Vec::new(),
        }
    }
}

/// A complete theme: every role has a color.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Name, kind, and description.
    pub meta: Meta,
    pub(crate) colors: [Rgb; ColorRole::COUNT],
    pub(crate) styles: [Style; StyleRole::COUNT],
    /// Which roles were derived at load rather than given by the file.
    pub(crate) derived: [bool; ColorRole::COUNT],
    /// Highlight colors offered to the reader, in menu order.
    pub user_highlights: Vec<UserHighlight>,
    /// Unknown keys in `[colors]`.
    pub colors_extra: Table,
    /// Unknown keys in `[styles]`.
    pub styles_extra: Table,
    /// Unknown top-level keys.
    pub extra: Table,
}

impl Theme {
    /// The theme's settings name.
    pub fn name(&self) -> &str {
        &self.meta.name
    }

    /// Light, dark, or high contrast.
    pub fn kind(&self) -> ThemeKind {
        self.meta.kind
    }

    /// One color.
    pub fn color(&self, role: ColorRole) -> Rgb {
        self.colors[role.index()]
    }

    /// Changes one color. The role counts as given from then on, so a
    /// copied file keeps it.
    pub fn set_color(&mut self, role: ColorRole, color: Rgb) {
        self.colors[role.index()] = color;
        self.derived[role.index()] = false;
    }

    /// True when this role was worked out at load (one of
    /// [`ColorRole::DERIVED`] that the file did not give).
    pub fn is_derived(&self, role: ColorRole) -> bool {
        self.derived[role.index()]
    }

    /// One style as stored.
    pub fn style(&self, role: StyleRole) -> &Style {
        &self.styles[role.index()]
    }

    /// One style, to change it.
    pub fn style_mut(&mut self, role: StyleRole) -> &mut Style {
        &mut self.styles[role.index()]
    }

    /// True when the background is darker than the text (dark and most
    /// high-contrast themes).
    pub fn is_dark(&self) -> bool {
        self.color(ColorRole::Background).relative_luminance()
            < self.color(ColorRole::Text).relative_luminance()
    }

    /// A style as it appears: reverse video applied, page filled in.
    pub fn resolve_style(&self, role: StyleRole) -> Resolved {
        self.resolve(self.style(role))
    }

    /// Resolves any style against this theme's page.
    pub fn resolve(&self, style: &Style) -> Resolved {
        let page = self.color(ColorRole::Background);
        let bg = style.background.unwrap_or(page);
        let (foreground, background, has_band) = if style.attributes.reverse {
            (bg, style.foreground, true)
        } else {
            (style.foreground, bg, style.background.is_some())
        };
        Resolved {
            foreground,
            background,
            attributes: Attrs {
                reverse: false,
                ..style.attributes
            },
            has_band,
        }
    }

    /// The user highlight with this name (case-insensitive).
    pub fn user_highlight(&self, name: &str) -> Option<&UserHighlight> {
        self.user_highlights
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
    }

    /// A flat table of every color the GUI needs, as `(key, color)` pairs in
    /// a stable order: each `[colors]` key; `<style>.foreground` and
    /// `<style>.background` for each style as it appears (reverse video
    /// applied, the page filled in where there is no band); and
    /// `highlight.<name>.foreground` / `.background` for user highlights.
    pub fn rgb_table(&self) -> Vec<(String, Rgb)> {
        let mut v: Vec<(String, Rgb)> = ColorRole::ALL
            .iter()
            .map(|&r| (r.key().to_owned(), self.color(r)))
            .collect();
        for &r in StyleRole::ALL {
            let s = self.resolve_style(r);
            v.push((format!("{}.foreground", r.key()), s.foreground));
            v.push((format!("{}.background", r.key()), s.background));
        }
        for h in &self.user_highlights {
            let s = self.resolve(&h.style);
            v.push((format!("highlight.{}.foreground", h.name), s.foreground));
            v.push((format!("highlight.{}.background", h.name), s.background));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_keys_round_trip() {
        for &r in ColorRole::ALL {
            assert_eq!(ColorRole::from_key(r.key()), Some(r));
        }
        for &r in StyleRole::ALL {
            assert_eq!(StyleRole::from_key(r.key()), Some(r));
        }
        assert_eq!(ColorRole::COUNT, 30);
        assert_eq!(ColorRole::DERIVED.len(), 15);
        for &r in ColorRole::ALL {
            assert_eq!(r.is_derived(), ColorRole::DERIVED.contains(&r), "{r}");
        }
        assert_eq!(StyleRole::COUNT, 9);
        assert_eq!(ColorRole::heading(3), Some(ColorRole::Heading3));
        assert_eq!(ColorRole::heading(7), None);
    }

    #[test]
    fn no_color_attributes_keep_co_occurring_highlights_apart() {
        for &r in StyleRole::ALL {
            assert!(!r.no_color_attributes().is_empty(), "{r}");
        }
        assert_ne!(
            StyleRole::SpokenWord.no_color_attributes(),
            StyleRole::SpokenSentence.no_color_attributes()
        );
        assert_ne!(
            StyleRole::FindHit.no_color_attributes(),
            StyleRole::CurrentFindHit.no_color_attributes()
        );
    }

    #[test]
    fn attrs_names() {
        let a = Attrs::BOLD.with(Attrs::REVERSE);
        assert_eq!(a.names(), ["bold", "reverse"]);
        assert_eq!(Attrs::from_name("Underlined"), Some(Attrs::UNDERLINE));
        assert_eq!(Attrs::from_name("blink"), None);
        assert!(Attrs::NONE.is_empty());
    }

    #[test]
    fn kind_keys() {
        for k in [ThemeKind::Light, ThemeKind::Dark, ThemeKind::HighContrast] {
            assert_eq!(ThemeKind::from_key(k.key()), Some(k));
        }
        assert_eq!(
            ThemeKind::from_key("high_contrast"),
            Some(ThemeKind::HighContrast)
        );
    }
}
