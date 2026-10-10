//! Integration tests of file type detection.

use diroxide_config::FileTypesConfig;
use diroxide_filetype::TypeRegistry;

#[test]
fn user_types_from_toml_join_the_registry() {
    let config = FileTypesConfig::from_toml(
        "[[type]]\nid = \"ebook\"\nname = \"EPUB book\"\nshort = \"EPUB\"\nextensions = [\"epub\"]\npriority = 5\n",
    )
    .unwrap();
    let types = TypeRegistry::builtin().with_config(&config);
    assert_eq!(types.types()[0].id, "ebook");
    assert_eq!(types.detect("novel.EPUB", &[]).unwrap().type_id, "ebook");
}
