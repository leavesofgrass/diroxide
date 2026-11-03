# Themes and colors

This guide covers the 24 themes that come with diroxide, how to choose one, what the words "meets AA" and "below AA" mean, the ten themes that fall short and what the window does for them, your own colors in the `[colors]` table, your own theme files, Windows' High Contrast, and following the system's light or dark setting. The terminal and the window draw the same 24 themes. Where they differ, such as for your own theme files, this guide says so.

What this guide quotes is what the program says, taken from the program's code and tests. Nobody has listened to the lines that announce a theme on a screen reader or a Braille display yet. If your experience differs, that is a finding.

Color never carries meaning alone, in any theme. A row says "folder", "link", "broken link", "hidden" and "selected" in words. Every part of the screen also keeps its own bold, underline or italic, so a theme changes how things look and never what they say. The six label colors are never red or green in any theme, and a test checks that.

## Choosing a theme

- Settings (Alt+T, then S, or Ctrl+Comma in the window) has a row, Theme (`theme`), whose values are the 24 themes, then any theme files of your own, then "Back to default". The change is saved at once, and the program says the theme's contrast verdict.
- `diro --theme NAME` chooses one for one session. NAME is the name in code type below, such as `galaxy-light`. An unknown name gives Galaxy. The terminal also says "Unknown theme: NAME. Galaxy is in use." before it opens. The window says nothing for an unknown name and draws Galaxy.
- Next theme, in the View menu (Alt+V, then T) and the palette, steps to the next theme in the order below, through your own theme files after the 24, and back to the first.
- The default is `galaxy`.

## What is said when you choose one

The program measures every theme for contrast and says the result in words, theme first:

```text
Theme Galaxy Light, meets AA
Theme Nord, below AA: 1 check falls short
Theme One Light, below AA: 8 checks fall short
```

- The text of a theme must reach 4.5 to 1 against its background. That is level AA. A part such as the focus ring must reach 3 to 1.
- The two high-contrast themes, High Contrast and Contrast, are held to level AAA, which is 7 to 1. For them the words are "meets AAA".
- A check is one pair of colors measured against its floor. "Below AA: 8 checks fall short" counts the pairs that miss.
- The line is said once, when you change the theme, in the terminal and in the window. Starting with a theme says nothing about it. When you choose a theme in the Settings list, the verdict takes the place of the plain "Theme: nord" line, so you do not hear both, and Next theme says the same line. Choosing Back to default says the default's: "Theme Galaxy, meets AA".
- A theme that is below AA is followed in the window by how many parts of the screen use the text color instead of their own: "In the window, 19 roles use the text color." With one part it says "1 role uses".

## The 24 themes

The order here is the order Next theme takes. The 14 themes that meet the floor come first, light and dark partners side by side. The ten that fall short come last, also with partners side by side.

### The 14 that meet the floor

Choosing one of these says "meets AA", or "meets AAA" for the two high-contrast themes. The window draws every part in the theme's own colors.

- `galaxy`: Galaxy, the default. A dark theme with purple accents.
- `galaxy-light`: Galaxy Light. Galaxy's light partner, purple accents on white.
- `high-contrast`: High Contrast. A softened high contrast for low vision, with every color at 7 to 1 or more.
- `contrast`: Contrast. Pure yellow and cyan on black, at 7 to 1 or more.
- `lamplight`: Lamplight. A soft, warm dark theme with low glare. Its body text reaches 7 to 1, but the theme is a dark one, so it is held to AA.
- `sepia`: Sepia. Warm paper tones for long reading, light.
- `gruvbox-dark`: Gruvbox Dark. A retro warm dark palette.
- `gruvbox-light`: Gruvbox Light. Gruvbox's light partner.
- `tokyo-night`: Tokyo Night. The Tokyo Night palette, dark.
- `catppuccin-mocha`: Catppuccin Mocha. The pastel palette, dark.
- `dracula`: Dracula. The Dracula palette, dark.
- `rose-pine`: Rose Pine. A muted rose and pine palette, dark.
- `kanagawa`: Kanagawa. A palette after Hokusai's wave, dark.
- `amber`: Amber. Low-blue amber on near black.

### The 10 that are below AA

Choosing one of these says "below AA" and how many checks fall short. Each of the ten keeps the colors it came with. None of the ten is the default.

- `one-dark`: One Dark. The One Dark editor palette, dark.
- `one-light`: One Light. Its light partner. It falls short the most.
- `dark`: Dark. A plain original dark theme.
- `light`: Light. Its light partner.
- `solarized-dark`: Solarized Dark.
- `solarized-light`: Solarized Light.
- `nord`: Nord. The Nord arctic palette, dark.
- `everforest-dark`: Everforest Dark. A soft green forest palette.
- `monokai`: Monokai. The Monokai palette, dark.
- `phosphor`: Phosphor. Green phosphor on black.

## What the window does for the ten

The window checks every pair of colors it draws. A part of the screen whose own color fails the floor on its background is drawn in the theme's text color instead. Where even the text color fails there, it is drawn in black or white, whichever reads better. The part keeps its bold, underline or italic, and its words. Because of this, the window never draws a pair below the floor, in any of the 24 themes, and a test checks every theme.

The terminal draws a theme's colors as they are, and says the verdict. Only the window changes a color that falls short.

What each of the ten needs in the window, as the alpha 3 build measures it. The window tells you the number whenever you choose the theme.

- One Light: 19 parts use the text color. It is the most of any theme.
- Light: 11 parts.
- One Dark: 7 parts.
- Solarized Dark, Nord, Everforest Dark and Monokai: 4 parts each. They are broken links, removed lines in a compare, "will be removed" rows in a delete plan, and problems in code.
- Phosphor: 4 parts.
- Dark: 3 parts. They are hidden entries, the inactive title line, and comments in code.
- Solarized Light: none. The theme's own checks say it falls short, but every pair the window draws already meets the floor.

## Your own colors

The `[colors]` table in `config.toml` sets your own color for any part of the screen. A part is a role, such as `folder` or `broken_link`, and a color is `#rrggbb`. [The settings reference](settings-reference.md#color-roles) lists every role.

```toml
[colors]
folder = "#ffaa00"
```

- The role keeps its bold, underline or italic. Your color replaces only its text color.
- Each color is checked against the role's background at the theme's floor, and the verdict counts a weak one: "Theme Galaxy, below AA: 1 check falls short".
- In the window, a color of yours that falls short is not drawn. The role is drawn in the text color, and the window says "In the window, 1 role uses the text color."
- A role name diroxide does not know is kept as it is and reported at start: "Unknown color role: NAME. Kept as it is." A color that is not `#rrggbb` is reported too: `colors.folder cannot be "orange": write it as #rrggbb.`
- The six label colors, `label_blue`, `label_orange`, `label_purple`, `label_yellow`, `label_gray` and `label_brown`, are roles like the others.

## Windows' High Contrast

Windows' High Contrast, which its settings call Contrast themes, is the person's own choice of colors. While it is on, the window draws only with the system's colors, and it wins over every theme and over your `[colors]`.

- The page is the system's window color, and text is its window text color.
- The focused row, marked rows, search matches, the active title line and a dialog's default button use the system's highlight colors.
- Links use the system's link color. The key bar and dialogs use the button colors. Hidden and unavailable entries use the system's gray text color.
- Every other part is plain text, and keeps its bold, underline or italic.
- The window reads the setting at every step, so turning High Contrast on or off applies at once. When the system's colors take over while the window is open, it says "System high contrast colors in use." At the start it says nothing about the colors. When High Contrast is turned off, your theme comes back.
- The window honors the system's colors as they are, even where one pair is a little under 7 to 1. Windows' Desert is one: its highlight is 6.9 to 1.

On Linux and macOS the system's high contrast setting is read once, when the window starts. It chooses the High Contrast theme, unless you have chosen another theme in that session.

## Following the system's light or dark setting

The setting `follow_system_appearance` ("Follow the system", off by default) lets the system choose between a theme and its partner.

- With it on, and the system in light mode, a dark theme gives way to its light partner, and the other way round. The partners are Galaxy and Galaxy Light, Gruvbox Dark and Gruvbox Light, One Dark and One Light, Dark and Light, and Solarized Dark and Solarized Light.
- A theme with no partner is replaced by Galaxy in dark mode, Galaxy Light in light mode, and High Contrast in a high contrast mode. A theme that already matches the system is kept.
- The window reads the system's setting once, when it starts. It asks the system with the system's own tools: `reg query` on Windows, `defaults` on macOS, and `gsettings` and the `GTK_THEME` variable on Linux. If it cannot tell, it keeps your theme.
- A theme you choose during the session ends the following for that session.
- With the setting off, your theme is the theme, whatever the system says.

## Your own themes

A theme of your own is a TOML file in the `themes` folder of the config folder. The program reads that folder when it starts and finds each theme by the `name` in the file's `[theme]` table. A theme of yours is listed in the Settings theme chooser after the 24 built-in themes, before Back to default, and Next theme steps through it after the last built-in theme and goes back to the first. Choosing it says its contrast verdict like any other: "Theme Harbor, meets AA". A file that cannot be read is left out, and the start says so once, in a line that begins, for a file named `broken.toml`, "Theme file broken.toml was not loaded".

The terminal draws your themes. The window does not read the `themes` folder in this alpha, so it draws only the 24 built-in themes. For a name it does not know, which includes your themes, it draws Galaxy and does not say that the name is unknown. Use the terminal, `diro --tui`, for your own themes for now.

One more known limit: `diro --theme harbor` says "Unknown theme: harbor. Galaxy is in use." before the terminal opens, even when `harbor.toml` is in your `themes` folder, because that sentence knows only the built-in names. The terminal then draws your theme. Choosing the theme in Settings does not say it.

[Settings and where the files live](settings.md#themes-and-colors) has the rest. The built-in theme files in the source, in `crates/diroxide-theme/themes`, are models to copy: copy one into your folder under a new name and change its `theme.name`.

## In a terminal

`NO_COLOR` is honored: with it set, the terminal uses bold, underline and reverse alone. The `borders` setting (off) turns on box drawing for sighted users. [Terminals](terminals.md) has the rest.

## See also

- [The GUI](gui.md)
- [Settings and where the files live](settings.md)
- [The settings reference](settings-reference.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [The accessibility statement](accessibility.md)
- [Terminals](terminals.md)
- [The documentation index](README.md)
