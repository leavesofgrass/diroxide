# The diroxide logo

Approved by the owner on Saturday, October 10, 2026, from the first round of concepts (concept B and the wordmark). The source files, the other concepts and the designer's full descriptions are in the planning repository under `design/logo/`.

## Files

- `diroxide-mark.svg`: the mark, used for the application icon, the favicon and everything small. A 32-unit drawing with 1.5-unit outlines, so it reads at 32 and 16 pixels. The window wave rasterizes it with `cargo xtask icons` into PNG sizes, a Windows `.ico`, a macOS `.icns` and a Linux `.desktop` icon.
- `diroxide-illustration.svg`: the full drawing of the same concept, for the README, the website and anything 64 pixels or larger. Never shrink it below 64 pixels; use the mark instead.
- `diroxide-wordmark.svg`: the word "diroxide" with the tagline "Directory Oxide: a modern Rust file manager" below it.

## What the mark shows

A rusty steel-blue filing cabinet seen from the front right and a little above, its top drawer pulled toward the viewer. A friendly orange crab sits on the open drawer's edge, one claw resting on the drawer, the other raised high in a wave. Cel-shaded: flat fills, one hard-edged shade step per shape, off-white highlights, dark ink outlines, no gradients. The illustration adds the crab's legs dangling over the drawer, an open smile, two file folders standing in the drawer, rust drips, cartoon wave marks and a pale halftone backdrop.

Alt text for the mark and the illustration: "diroxide logo: a friendly orange crab sitting on the open drawer of a rusty steel-blue filing cabinet, waving one claw."

Alt text for the wordmark: "diroxide, Directory Oxide: a modern Rust file manager."

## The wordmark

The letters of "diroxide" are bold, round-ended, geometric shapes drawn as stroked paths, so no font is needed: "dir" in steel blue, "oxide" in rust orange, each letter with an ink outline and a hard drop shadow. The tagline is text set in a hyperlegible sans-serif with a plain fallback, fixed to the word's width. If the two-color split ever reads as two words, the fallback is one color, ink outline and rust-orange fill.

## Palette

Rust orange `#e2742a` against steel blue `#3f6e9e`, with ink `#1b1f2b` for every outline and off-white `#f6f1e7` for highlights and the eyes; shades rust `#b0521b`, deep rust `#7c3312`, steel shade `#2b4f75`, steel light `#7ea6cc`, drawer dark `#252c3c`. No red and no green anywhere; the pair sits on the blue-yellow axis, which red-green color blindness leaves intact. Color never carries meaning: the logo means the same thing in grayscale, where the ink outlines do the separating.

## The crab

Drawn from the brief's words (friendly, rounded, big claws, small eyes): a dome shell, mitten-shaped pincers on thick arms, small eyes on stalks. It is a nod to Rust's crab mascot and is not a copy of the Ferris artwork.

## Still to do

- Nothing here has been rendered yet; a sighted review or a raster render comes before the first release that ships the icon.
- The claws' notch is about one pixel at 16 pixels and may fill in; check it after rasterizing.
