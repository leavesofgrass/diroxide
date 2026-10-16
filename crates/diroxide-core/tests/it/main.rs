//! Integration tests of the core model's public surface.

mod props;

use diroxide_core::place::PathStyle;
use diroxide_core::{
    ArgValue, Command, CommandId, Effect, Entry, EntryKind, KeyChord, ListModel, ListRow, Message,
    Place, Redraw,
};

#[test]
fn a_listing_reads_as_rows_with_the_position_last() {
    let entries = [
        Entry::new("notes.txt", EntryKind::File).with_size(1_200),
        Entry::new("week1", EntryKind::Folder),
        Entry::new("odd\u{202E}name", EntryKind::File),
    ];
    let rows = entries
        .iter()
        .map(|e| {
            ListRow::new(e.name.clone(), e.display_name()).with_details([e.kind.word().to_owned()])
        })
        .collect();
    let place = Place::parse_with("D:\\docs\\course.zip!", PathStyle::Windows).unwrap();
    let mut list = ListModel::new(place.title()).with_rows(rows);
    assert_eq!(list.title(), "course.zip, archive, in D:\\docs");
    assert!(list.letter_jump('o'));
    assert_eq!(list.announce_focus(), "odd?name, file, 3 of 3");
    assert_eq!(list.focused().unwrap().id, "odd\u{202E}name");
}

#[test]
fn commands_and_effects_travel_as_json() {
    let place = Place::parse_with("/home/ada/a.zip!x", PathStyle::Unix).unwrap();
    let command = Command::new(CommandId::from_static("file.copy"))
        .with_arg("sources", ArgValue::Places(vec![place]));
    if cfg!(not(windows)) {
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(
            json,
            r#"{"id":"file.copy","args":{"sources":{"places":["/home/ada/a.zip!x"]}}}"#
        );
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, command);
    }
    let effects = vec![
        Effect::Redraw(Redraw::All),
        Effect::Announce(Message::done("Copied.")),
    ];
    let json = serde_json::to_string(&effects).unwrap();
    let back: Vec<Effect> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, effects);
    assert!(serde_json::from_str::<CommandId>(r#""Not.Valid""#).is_err());
}

#[test]
fn chords_read_from_text() {
    let chord: KeyChord = serde_json::from_str(r#""ctrl+x ctrl+f""#).unwrap();
    assert_eq!(chord.to_string(), "Ctrl+X Ctrl+F");
}
