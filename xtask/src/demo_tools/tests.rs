//! The demo set's own tests: every fixture made, the same bytes twice,
//! plain names only, and each format read back by hand.

use super::*;

/// A new folder for one test, under the build's target folder, so the
/// set is never made anywhere else.
pub(super) fn temp() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let work = target_folder(&root).join("demo-tools-tests");
    fs::create_dir_all(&work).unwrap();
    tempfile::Builder::new()
        .prefix("demo-tools-")
        .tempdir_in(&work)
        .unwrap()
}

/// The set made without git, in a new folder.
pub(super) fn make_set() -> (tempfile::TempDir, PathBuf, Made) {
    let dir = temp();
    let folder = dir.path().join("check");
    let staging = dir.path().join("staging");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(&staging).unwrap();
    let made = build(&folder, &staging, false).unwrap();
    (dir, folder, made)
}

#[test]
fn every_fixture_is_made_with_plain_names_and_the_same_bytes_twice() {
    let (_one, first, made) = make_set();
    assert!(made.all_good);
    // One line a fixture, the name first; git's two said left out here.
    for (line, name) in made.lines.iter().zip(FIXTURES) {
        assert!(line.starts_with(&format!("{name}, ")), "{line}");
    }
    assert!(made.lines[FIXTURES.len()].starts_with(STEPS_FILE));
    assert_eq!(
        made.lines[FIXTURES.len() + 1],
        format!("Step 2's shell line: {}", rename_line(&first))
    );
    assert_eq!(
        made.lines[FIXTURES.len() + 2],
        format!("Step 8's cmd.exe line: {}", pipe_line(&first))
    );
    assert_eq!(
        made.lines.last().unwrap(),
        &format!("Start the check: {}", start_line(&first))
    );
    assert_eq!(made.lines.len(), FIXTURES.len() + 4);
    let (_two, second, _) = make_set();
    let files = every_file(&first);
    assert_eq!(files, every_file(&second));
    // Alpha 4's 28, and alpha 5's 101 (100 where no hard link is made).
    let linked = first.join("dups/linked/two.txt").is_file();
    assert_eq!(files.len(), if linked { 129 } else { 128 }, "{files:?}");
    for file in &files {
        for part in file.components() {
            let name = part.as_os_str().to_str().unwrap();
            assert!(crate::demo::plain(name), "{}", file.display());
        }
        // The task file, the steps, the saved files and the pipe's list
        // name their own folder; everything else is the same.
        if file.ends_with("scans.toml")
            || file.ends_with(STEPS_FILE)
            || file.starts_with("state")
            || file.ends_with("paths.nul")
        {
            continue;
        }
        assert!(
            fs::read(first.join(file)).unwrap() == fs::read(second.join(file)).unwrap(),
            "{} differs between two runs",
            file.display()
        );
        let a = fs::metadata(first.join(file)).unwrap().modified().unwrap();
        let b = fs::metadata(second.join(file)).unwrap().modified().unwrap();
        assert_eq!(a, b, "{}", file.display());
    }
    for name in [
        "photos/broken.png",
        "photos/crow-on-the-wire.png",
        "mixed/portrait.jpg",
        "mixed/liar.png",
        "mixed/cp1250.txt",
        "music/song.mp3",
        "docs/report.pdf",
        "docs/report.md",
        "docs/notes.org",
        "download/example-tool.zip",
        "download/example-notes.txt",
        "download/SHA256SUMS",
        "download/example-tool.zip.sha256",
        "disk/example.iso",
        "disk/two-parts.img",
        "hot/scans.toml",
    ] {
        assert!(first.join(name).is_file(), "{name}");
    }
    for name in [
        "hot/scans/in",
        "hot/scans/out",
        "hot/scans/processed",
        "hot/scans/failed",
        "out",
    ] {
        assert!(first.join(name).is_dir(), "{name}");
    }
}

#[test]
fn the_checksum_lists_say_one_match_one_stale_and_one_missing() {
    let (_dir, folder, _) = make_set();
    let download = folder.join("download");
    let zip = sha256_hex(&fs::read(download.join("example-tool.zip")).unwrap());
    let notes = sha256_hex(&fs::read(download.join("example-notes.txt")).unwrap());
    let sums = fs::read_to_string(download.join("SHA256SUMS")).unwrap();
    let lines: Vec<&str> = sums.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], format!("{zip}  example-tool.zip"));
    assert!(lines[1].ends_with("  example-notes.txt") && !lines[1].starts_with(&notes));
    assert!(lines[2].ends_with("  example-extra.txt"));
    assert!(!download.join("example-extra.txt").exists());
    assert_eq!(
        fs::read_to_string(download.join("example-tool.zip.sha256")).unwrap(),
        format!("{zip}  example-tool.zip\n")
    );
    // The zip holds the two files, read back by the zip crate.
    let mut reader =
        zip::ZipArchive::new(fs::File::open(download.join("example-tool.zip")).unwrap()).unwrap();
    let mut names: Vec<String> = (0..reader.len())
        .map(|i| reader.by_index(i).unwrap().name().unwrap().into_owned())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["README.txt", "tool.txt"]);
}

/// A big-endian TIFF's IFD at `at`: each tag with its type, count and
/// value field's four bytes.
fn read_ifd(tiff: &[u8], at: usize) -> Vec<(u16, u16, u32, [u8; 4])> {
    let u16_at = |i: usize| u16::from_be_bytes([tiff[i], tiff[i + 1]]);
    let u32_at = |i: usize| u32::from_be_bytes([tiff[i], tiff[i + 1], tiff[i + 2], tiff[i + 3]]);
    let count = usize::from(u16_at(at));
    (0..count)
        .map(|n| {
            let e = at + 2 + 12 * n;
            (
                u16_at(e),
                u16_at(e + 2),
                u32_at(e + 4),
                [tiff[e + 8], tiff[e + 9], tiff[e + 10], tiff[e + 11]],
            )
        })
        .collect()
}

/// An ASCII value's text, from its offset.
fn ascii(tiff: &[u8], entry: &(u16, u16, u32, [u8; 4])) -> String {
    let len = entry.2 as usize;
    let bytes = if len <= 4 {
        entry.3[..len].to_vec()
    } else {
        let at = u32::from_be_bytes(entry.3) as usize;
        tiff[at..at + len].to_vec()
    };
    String::from_utf8(bytes)
        .unwrap()
        .trim_end_matches('\0')
        .to_owned()
}

#[test]
fn the_portrait_is_stored_wide_with_its_exif_and_the_liar_is_a_jpeg() {
    let (_dir, folder, _) = make_set();
    let portrait = fs::read(folder.join("mixed/portrait.jpg")).unwrap();
    assert_eq!(&portrait[..4], &[0xFF, 0xD8, 0xFF, 0xE1]);
    assert_eq!(&portrait[6..12], b"Exif\0\0");
    let tiff = &portrait[12..];
    assert_eq!(&tiff[..8], b"MM\x00\x2a\x00\x00\x00\x08");
    let first = read_ifd(tiff, 8);
    let tags: Vec<u16> = first.iter().map(|e| e.0).collect();
    assert_eq!(tags, [0x010E, 0x010F, 0x0110, 0x0112, 0x8769, 0x8825]);
    assert_eq!(ascii(tiff, &first[0]), "Crow on a wire");
    assert_eq!(ascii(tiff, &first[1]), "Example");
    assert_eq!(ascii(tiff, &first[2]), "Example Camera");
    assert_eq!(&first[3].3[..2], &6u16.to_be_bytes());
    let exif = read_ifd(tiff, u32::from_be_bytes(first[4].3) as usize);
    assert_eq!(ascii(tiff, &exif[0]), "2026:10:11 09:30:00");
    assert_eq!(ascii(tiff, &exif[1]), "EX-0001234");
    let gps = read_ifd(tiff, u32::from_be_bytes(first[5].3) as usize);
    assert_eq!(gps.len(), 5);
    assert_eq!(ascii(tiff, &gps[1]), "N");
    // Stored 400 by 300; shown 300 by 400 by its orientation.
    let picture = image::load_from_memory(&portrait).unwrap();
    assert_eq!((picture.width(), picture.height()), (400, 300));
    let liar = fs::read(folder.join("mixed/liar.png")).unwrap();
    assert_eq!(&liar[..3], &[0xFF, 0xD8, 0xFF]);
    assert_ne!(&liar[3], &0xE1, "the liar has no Exif");
    // Polish text in Windows-1250: not UTF-8, and its letters where 1250
    // puts them.
    let text = fs::read(folder.join("mixed/cp1250.txt")).unwrap();
    assert!(String::from_utf8(text.clone()).is_err());
    assert_eq!(&text[..6], &[b'Z', b'a', 0xBF, 0xF3, 0xB3, 0xE6]);
}

#[test]
fn the_song_is_twelve_seconds_of_silent_frames_after_its_tags() {
    let song = song_mp3();
    assert_eq!(&song[..3], b"ID3");
    let size = song[6..10]
        .iter()
        .fold(0usize, |n, b| (n << 7) | usize::from(*b));
    let audio = &song[10 + size..];
    assert_eq!(audio.len(), SONG_FRAMES * FRAME_BYTES);
    for frame in audio.chunks(FRAME_BYTES) {
        assert_eq!(&frame[..4], &[0xFF, 0xFB, 0x90, 0x00]);
        assert!(
            frame[4..].iter().all(|b| *b == 0),
            "a frame that could sound"
        );
    }
    let seconds = SONG_FRAMES as f64 * 1152.0 / 44_100.0;
    assert!((12.0..12.1).contains(&seconds), "{seconds}");
    let tags = String::from_utf8_lossy(&song[10..10 + size]);
    for text in ["TIT2", "Example Song", "TPE1", "Ada Example", "TALB"] {
        assert!(tags.contains(text), "{text}");
    }
}

#[test]
fn the_pdfs_cross_references_point_at_its_objects() {
    let pdf = report_pdf();
    // One character a byte, so offsets in the text are the file's: the
    // header comment's four binary bytes read as question marks.
    let text: String = pdf
        .iter()
        .map(|b| if b.is_ascii() { char::from(*b) } else { '?' })
        .collect();
    assert!(text.starts_with("%PDF-1.4\n%????\n"));
    assert!(text.contains("/Count 3"));
    assert!(text.contains("/Title (Example Report) /Author (Ada Example)"));
    let start: usize = text
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(text[start..].starts_with("xref\n0 11\n"));
    let entries: Vec<&str> = text[start..].lines().skip(3).take(10).collect();
    for (n, entry) in entries.iter().enumerate() {
        assert_eq!(entry.len(), 19, "{entry:?}");
        let offset: usize = entry[..10].parse().unwrap();
        assert!(
            text[offset..].starts_with(&format!("{} 0 obj\n", n + 1)),
            "object {}",
            n + 1
        );
    }
}

#[test]
fn the_iso_has_both_trees_and_rock_ridge() {
    let iso = example_iso();
    assert_eq!(iso.len(), 31 * SECTOR);
    let sector = |n: usize| &iso[n * SECTOR..(n + 1) * SECTOR];
    let primary = sector(16);
    assert_eq!(&primary[..7], b"\x01CD001\x01");
    assert_eq!(&primary[40..47], b"EXAMPLE");
    assert_eq!(u32::from_le_bytes(primary[80..84].try_into().unwrap()), 31);
    let joliet = sector(17);
    assert_eq!(&joliet[..7], b"\x02CD001\x01");
    assert_eq!(&joliet[88..91], b"%/E");
    assert_eq!(&sector(18)[..7], b"\xffCD001\x01");
    // The primary root's own record: Rock Ridge's SP first, then PX and
    // the extension reference.
    let root_extent = u32::from_le_bytes(primary[158..162].try_into().unwrap()) as usize;
    let root = sector(root_extent);
    let own = &root[..usize::from(root[0])];
    assert_eq!(own[32], 1);
    assert_eq!(&own[34..41], &[b'S', b'P', 7, 1, 0xBE, 0xEF, 0]);
    assert!(own.windows(10).any(|w| w == b"RRIP_1991A"));
    // Its children: EFI and README.TXT;1 with their Rock Ridge names.
    let mut at = usize::from(root[0]);
    at += usize::from(root[at]);
    let mut names = Vec::new();
    while root[at] != 0 {
        let len = usize::from(root[at]);
        let id_len = usize::from(root[at + 32]);
        names.push(String::from_utf8(root[at + 33..at + 33 + id_len].to_vec()).unwrap());
        assert!(root[at..at + len].windows(2).any(|w| w == b"NM"));
        at += len;
    }
    assert_eq!(names, ["EFI", "README.TXT;1"]);
    // The files' bytes where their records point.
    assert!(sector(30).starts_with(README_BYTES));
    assert!(sector(29).starts_with(BOOT_FILE_BYTES));
    // Joliet's root lists the names in UCS-2.
    let jroot_extent = u32::from_le_bytes(joliet[158..162].try_into().unwrap()) as usize;
    let jroot = sector(jroot_extent);
    assert!(jroot.windows(6).any(|w| w == ucs2("EFI").as_slice()));
}

#[test]
fn the_disk_has_two_fat16_partitions_each_with_hello_txt() {
    let disk = two_parts_img();
    assert_eq!(disk.len(), DISK_SECTORS as usize * 512);
    assert_eq!(&disk[510..512], &[0x55, 0xAA]);
    for (i, (label, start)) in [("BOOT", PART_STARTS[0]), ("DATA", PART_STARTS[1])]
        .into_iter()
        .enumerate()
    {
        let entry = &disk[446 + 16 * i..462 + 16 * i];
        assert_eq!(entry[4], 0x06);
        assert_eq!(u32::from_le_bytes(entry[8..12].try_into().unwrap()), start);
        assert_eq!(
            u32::from_le_bytes(entry[12..16].try_into().unwrap()),
            PART_SECTORS
        );
        let part = &disk[start as usize * 512..(start + PART_SECTORS) as usize * 512];
        assert_eq!(&part[54..62], b"FAT16   ");
        assert_eq!(&part[43..47], label.as_bytes());
        assert_eq!(&part[510..512], &[0x55, 0xAA]);
        // FAT16 by Microsoft's count of clusters: 4,085 to 65,524.
        let data = PART_SECTORS - DATA_START;
        let clusters = data / 2;
        assert!((4085..65_525).contains(&clusters), "{clusters}");
        // The root folder: the label, hello.txt's long name, its short
        // name pointing at cluster 2, and the text there.
        let root = &part[65 * 512..];
        assert_eq!(&root[..4], label.as_bytes());
        assert_eq!(root[11], 0x08);
        assert_eq!(root[32 + 11], 0x0F);
        assert_eq!(root[32 + 13], short_name_sum(b"HELLO   TXT"));
        assert_eq!(&root[32 + 1..32 + 3], &[b'h', 0]);
        assert_eq!(&root[64..75], b"HELLO   TXT");
        let size = u32::from_le_bytes(root[64 + 28..64 + 32].try_into().unwrap()) as usize;
        let text = &part[DATA_START as usize * 512..DATA_START as usize * 512 + size];
        assert!(text.starts_with(b"Hello from partition"));
    }
}

/// What the app's details read and Say Details give for a fixture, with
/// no helper program found: the rows as "Label: value", then the summary.
fn heard(folder: &Path, name: &str) -> (Vec<String>, String) {
    use diroxide_app::details::{DetailsAsk, Wanted, read_fields, summary};
    use diroxide_app::request::SharedTools;
    use diroxide_vfs::{OpContext, Provider};
    let place = diroxide_core::Place::parse(folder.join(name).to_str().unwrap()).unwrap();
    let entry = diroxide_vfs::LocalProvider::new()
        .stat(&place, &OpContext::new())
        .unwrap();
    let ask = DetailsAsk {
        generation: 1,
        place,
        entry,
        file_type: None,
        tools: SharedTools(std::sync::Arc::new(diroxide_vfs::NoTools)),
        cache: None,
        wanted: Wanted::Fields,
    };
    let registry = diroxide_vfs::Registry::with_local();
    let (rows, _) = read_fields(&registry, &ask);
    let lines = rows
        .iter()
        .map(|r| format!("{}: {}", r.label, r.value))
        .collect();
    (lines, summary(&registry, &ask))
}

/// The fixtures read as R30's steps 1 to 3 say, through the app's own
/// readers (no helper program runs).
#[test]
fn the_app_reads_the_fixtures_as_the_steps_say() {
    let (_dir, folder, _) = make_set();
    let (rows, said) = heard(&folder, "mixed/portrait.jpg");
    eprintln!("portrait: {said} | {rows:?}");
    // R30's step 1 says "Taken: Sunday, October 11, 2026"; the build says
    // the date and time as written (the fold lane's report names it).
    for line in [
        "Camera: Example Camera",
        "Taken: 2026-10-11 09:30",
        "Description: Crow on a wire",
        "Dimensions: 300 by 400, as shown",
        "Location: ",
        "Camera serial: EX-0001234",
    ] {
        assert!(
            rows.iter().any(|r| r.starts_with(line)),
            "{line} in {rows:?}"
        );
    }
    assert!(said.starts_with("JPEG, 300 by 400"), "{said}");
    let (rows, said) = heard(&folder, "mixed/liar.png");
    eprintln!("liar: {said} | {rows:?}");
    // Read by its contents: a JPEG, whatever its name says (the app's type
    // check says "Name says PNG; contents say JPEG" from the same bytes).
    assert!(said.starts_with("JPEG, 400 by 300"), "{said} {rows:?}");
    let (rows, said) = heard(&folder, "music/song.mp3");
    eprintln!("song: {said} | {rows:?}");
    for line in [
        "Artist: Ada Example",
        "Title: Example Song",
        "Length: 12 sec",
        "Bit rate: 128 kbps",
        "Channels: stereo",
    ] {
        assert!(rows.iter().any(|r| r == line), "{line} in {rows:?}");
    }
    let (rows, said) = heard(&folder, "docs/report.pdf");
    eprintln!("report: {said} | {rows:?}");
    for line in ["Title: Example Report", "Pages: 3", "Author: Ada Example"] {
        assert!(rows.iter().any(|r| r == line), "{line} in {rows:?}");
    }
    // With no pdftotext, the quick view says which tool it needs.
    assert_eq!(said, "No preview: pdftotext not found");
    let (rows, said) = heard(&folder, "photos/broken.png");
    eprintln!("broken: {said} | {rows:?}");
}

#[test]
fn the_task_file_reads_as_a_task() {
    let home = Path::new(if cfg!(windows) {
        r"D:\check\hot\scans"
    } else {
        "/check/hot/scans"
    });
    let text = scans_task(home);
    let shape: diroxide_ops::hot::task::TaskShape = toml::from_str(&text).unwrap();
    assert_eq!(shape.name, "scans");
    assert_eq!(shape.recipe.as_deref(), Some("image-to-jpeg"));
    assert_eq!(shape.trigger.matches, ["*.png"]);
    assert_eq!(shape.trigger.home, home.display().to_string());
    assert_eq!(shape.run.after, "move");
    assert_eq!(shape.run.conflicts, "keep-both");
}

#[test]
fn the_steps_name_every_step_and_the_start() {
    let folder = Path::new(r"D:\dirocheck5");
    let text = steps_text(folder);
    for step in 0..=9 {
        assert!(text.contains(&format!("\nStep {step}, ")), "step {step}");
    }
    assert!(!text.contains("Step 10, "));
    assert!(text.contains(&format!("\nStart: {}\n", start_line(folder))));
    assert!(text.contains(&format!("From a shell: {}\n", rename_line(folder))));
    assert!(text.contains(&format!("In cmd.exe: {}\n", pipe_line(folder))));
    for mark in ["{PATH}", "{START}", "{RENAME}", "{PIPE}"] {
        assert!(!text.contains(mark), "{mark}");
    }
}

/// Every step ends with the lanes whose words it waits on, and every
/// line to hear fits a 40-cell line.
#[test]
fn every_step_names_its_lanes_and_every_quoted_line_fits_40_cells() {
    let text = steps_text(Path::new(r"D:\dirocheck5"));
    let steps: Vec<&str> = text.split("\n\n").skip(1).collect();
    assert_eq!(steps.len(), 10);
    for step in steps {
        let last = step.trim_end().lines().last().unwrap();
        assert!(last.starts_with("To be checked: "), "{step}");
    }
    let mut quoted = 0;
    for line in text.lines() {
        for (at, part) in line.split('"').enumerate() {
            if at % 2 == 1 {
                quoted += 1;
                assert!(part.chars().count() <= 40, "{part}");
            }
        }
    }
    assert!(quoted >= 50, "{quoted}");
}

/// git makes both repositories when it is there; only the placeholder
/// identity is in them, and nothing of the computer's configuration.
#[test]
fn git_makes_both_repositories_when_it_is_there() {
    if !git_present() {
        eprintln!("git not found: the demo repositories were not made");
        return;
    }
    let dir = temp();
    let folder = dir.path().join("check");
    let staging = dir.path().join("staging");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(&staging).unwrap();
    let made = build(&folder, &staging, true).unwrap();
    assert!(
        made.lines.iter().any(|l| l.starts_with("repo, ")),
        "{:?}",
        made.lines
    );
    assert!(folder.join("repo/.git").is_dir());
    assert!(folder.join("repo/notes.txt").is_file());
    let config = fs::read_to_string(folder.join("repo-hook/.git/config")).unwrap();
    assert!(config.contains(MISSING_MONITOR), "{config}");
    let config = fs::read_to_string(folder.join("repo/.git/config")).unwrap();
    assert!(!config.contains("[user]"), "{config}");
    // The one commit is the placeholder's, at the fixed time.
    let log = fs::read_to_string(folder.join("repo/.git/logs/HEAD")).unwrap();
    assert!(
        log.contains("Ada Example <ada@example.invalid> 1791711000 +0000"),
        "{log}"
    );
    assert_eq!(log.lines().count(), 1, "{log}");
}
