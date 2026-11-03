# Bundled fonts

textweaver ships three font families so that PDF export, EPUB export, and the GUI look the same on every machine and never fail for lack of a font. All three are under the SIL Open Font License 1.1 (OFL). Each folder holds the font files exactly as published upstream, the licence file exactly as published upstream, and a README with the source, version, and SHA-256 of every file.

- `atkinson-hyperlegible-next/`: Atkinson Hyperlegible Next, by the Braille Institute of America. The default text font for PDF output. Regular, Bold, Italic, Bold Italic.
- `atkinson-hyperlegible-mono/`: Atkinson Hyperlegible Mono, by the Braille Institute of America. The code font for PDF output. Regular, Bold, Italic, Bold Italic.
- `opendyslexic/`: OpenDyslexic, by Abbie Gonzalez. Offered for readers with dyslexia. Regular, Bold, Italic, Bold Italic.
- `lexend/`: the license and README of Lexend, by the Lexend Project. Lexend is not bundled: textweaver downloads it the first time a reader chooses it, after asking, and checks each file against the SHA-256 in that README. Only its license ships here, so it travels with every package as the bundled fonts' licenses do.

Only these four styles of each family are kept, to limit size. Total size of the 12 font files: 1,354,240 bytes (1.29 MiB).

## How textweaver uses them

The `textweaver-fonts` crate embeds these files with `include_bytes!` behind its cargo feature `bundled-fonts` (on by default). The PDF writer embeds a subset of the glyphs it uses in each PDF; the EPUB writer can embed the whole font files, with the licence, when asked; the GUI registers them privately for its own process (nothing is installed on the system).

## Licence obligations (SIL OFL 1.1)

- The fonts may be bundled, embedded, and redistributed with software, including commercial software, as long as each copy carries the copyright notice and the licence. The `OFL.txt` in each folder is that licence; the README at the root of the repository points here.
- The fonts must not be sold by themselves.
- Modified versions must not use a Reserved Font Name. OpenDyslexic reserves the name "OpenDyslexic". textweaver does not modify the font files; the PDF writer embeds glyph subsets, which the OFL FAQ treats as embedding in a document, not as a modified font.

## Updating

Download the new files from the same upstream locations, check the licence text has not changed, replace the files, update the SHA-256 lines in the family's README, and run `cargo test -p textweaver-fonts` (it checks every bundled face parses and carries the expected family name).

Line ends: the `OFL.txt` files use CR LF, as upstream publishes them. `.gitattributes` marks them `-text` so git keeps their bytes, and the SHA-256 values match the files in a checkout.
