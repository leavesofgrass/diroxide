//! The readers this crate builds. Each says which files it takes, by the
//! type check's id or group and, when the type is not checked yet, by the
//! name's extension; each reads only through the source it is given,
//! which the guard holds to its limits.

use std::sync::Arc;

use crate::reader::{ReadRequest, Reader};

pub mod archive;
pub mod audio;
pub mod text;

/// The crate's readers in the pipeline's order: the fast ones first.
pub(crate) fn builtin() -> Vec<Arc<dyn Reader>> {
    vec![
        Arc::new(audio::AudioTags),
        Arc::new(audio::AudioProperties),
        Arc::new(archive::ZipFacts),
        Arc::new(text::TextFacts),
    ]
}

/// Whether a request is for one of `types`, or, when the type is not
/// checked yet, for a name ending in one of `extensions` (lowercase,
/// with the dot).
pub(crate) fn takes(request: &ReadRequest<'_>, types: &[&str], extensions: &[&str]) -> bool {
    if let Some(id) = request.type_id {
        return types.contains(&id);
    }
    let name = request.entry.name.to_ascii_lowercase();
    extensions.iter().any(|ext| name.ends_with(ext))
}
