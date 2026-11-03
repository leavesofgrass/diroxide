# Lexend

Lexend is **not bundled**: textweaver downloads it the first time a reader chooses it, after asking ("Download the Lexend font, 206 KB, SIL Open Font License? y or n"). Only its license is kept here, so it ships with textweaver and goes into the packages' notices, as the bundled fonts' licenses do. The pinned files and their SHA-256 values are in `crates/textweaver-fonts/src/downloaded.rs` (`LEXEND`); a download that does not match them is refused.

- Designers: Bonnie Shaver-Troup, Thomas Jockin, Santiago Orozco, Héctor Gómez, and Nadine Chanine (the Lexend Project).
- Home page: https://www.lexend.com/
- License: SIL Open Font License 1.1 (`OFL.txt`, unchanged from upstream), with Reserved Font Name "RevReading Lexend". Copyright 2018 The Lexend Project Authors.
- Version: the static `ttf` build in the Lexend project's repository, googlefonts/lexend, at commit `cd26b9c2538d758138c20c3d2f10362ed613854b` (2022-09-22, "Build Lexend with HEXP axis"), the last commit that changed those files. The repository's newest commit, `7894f02b2e7eabc48595f1d4eff3b17b48c6e651` (2023-03-02), has the same files. Star pinned the same commit.
- Source: https://raw.githubusercontent.com/googlefonts/lexend/cd26b9c2538d758138c20c3d2f10362ed613854b/fonts/lexend/ttf/ and, for the license, https://raw.githubusercontent.com/googlefonts/lexend/cd26b9c2538d758138c20c3d2f10362ed613854b/OFL.txt
- Checked: 2026-10-01.
- Embedding: installable (OS/2 `fsType` 0).

## Files

Each line: file, size in bytes, SHA-256. Only the regular and bold faces are downloaded; Lexend has no italics.

- `Lexend-Regular.ttf`, 100,264, `e2082c28389e9871d5d77ae9163d5c0c1c259fe39bdac91f73d56a600b61f60f`
- `Lexend-Bold.ttf`, 105,564, `4509838acc21f2c066e9874c9c5eb52b9b8cdac1771a9c1fe9e7e2cda41ce082`
- `OFL.txt`, 4,436, `5da8505887d0fa7fe963445fd58852707fda34adfeb65af25c99d152bab285bd` (LF line ends, as upstream)

Total download: 205,828 bytes (206 KB).

The two font files are also in `fixtures/w7l/`, for the tests, which never download: a fake fetcher hands them out.
