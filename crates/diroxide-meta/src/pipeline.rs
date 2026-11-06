//! The pipeline: the readers in order, and the fields they give for one
//! file in one cost class.

use std::sync::Arc;

use diroxide_vfs::{OpContext, Provider};

use crate::field::Field;
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};

/// The readers, in the order they are asked.
#[derive(Clone, Default)]
pub struct Pipeline {
    readers: Vec<Arc<dyn Reader>>,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self.readers.iter().map(|r| r.id()).collect();
        f.debug_struct("Pipeline").field("readers", &ids).finish()
    }
}

impl Pipeline {
    /// A pipeline with no readers.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same pipeline with a reader added last.
    pub fn with(mut self, reader: Arc<dyn Reader>) -> Self {
        self.readers.push(reader);
        self
    }

    /// The readers' ids, in order.
    pub fn ids(&self) -> Vec<&'static str> {
        self.readers.iter().map(|r| r.id()).collect()
    }

    /// The fields of one file from every reader of `cost` that accepts it,
    /// each opened afresh through `provider`; a reader that fails gives
    /// its error in place of fields, so one bad reader never hides the
    /// rest. Naive: each runs inline, where the metadata lane runs it
    /// under the guard.
    pub fn read(
        &self,
        provider: &dyn Provider,
        request: &ReadRequest<'_>,
        cost: Cost,
        limits: &ReadLimits,
        ctx: &OpContext,
    ) -> Vec<(&'static str, Result<Vec<Field>, MetaError>)> {
        self.readers
            .iter()
            .filter(|r| r.cost() == cost && r.accepts(request))
            .map(|r| {
                let fields = provider
                    .open_seekable(request.place, ctx)
                    .map_err(MetaError::from)
                    .and_then(|mut source| r.read(source.as_mut(), request, limits));
                (r.id(), fields)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use diroxide_core::details::DetailGroup;
    use diroxide_core::{Entry, EntryKind, Place};
    use diroxide_vfs::{LocalProvider, ReadSeek};

    use super::*;

    /// A reader that counts the bytes of a text file.
    struct Bytes;

    impl Reader for Bytes {
        fn id(&self) -> &'static str {
            "bytes"
        }
        fn cost(&self) -> Cost {
            Cost::Background
        }
        fn accepts(&self, request: &ReadRequest<'_>) -> bool {
            request.entry.name.ends_with(".txt")
        }
        fn read(
            &self,
            source: &mut dyn ReadSeek,
            _request: &ReadRequest<'_>,
            limits: &ReadLimits,
        ) -> Result<Vec<Field>, MetaError> {
            let mut bytes = Vec::new();
            source
                .take(limits.max_bytes + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| MetaError::Unreadable(e.to_string()))?;
            Ok(vec![Field::new(
                "text.bytes",
                "Bytes",
                bytes.len().to_string(),
                DetailGroup::Size,
            )])
        }
    }

    /// The scratch folder sits under the build's target folder, never the
    /// system's temporary folder.
    #[test]
    fn the_pipeline_asks_the_readers_of_one_class_that_take_the_file() {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("meta-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("meta-")
            .tempdir_in(&base)
            .unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"twelve bytes").unwrap();
        let place = Place::parse(&path.display().to_string()).unwrap();
        let entry = Entry::new("notes.txt", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: Some("text"),
            group: Some("text"),
        };
        let pipeline = Pipeline::new().with(Arc::new(Bytes));
        assert_eq!(pipeline.ids(), ["bytes"]);
        let provider = LocalProvider::new();
        let ctx = OpContext::new();
        let limits = ReadLimits::default();
        let got = pipeline.read(&provider, &request, Cost::Background, &limits, &ctx);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1.as_ref().unwrap()[0].value, "12");
        assert!(
            pipeline
                .read(&provider, &request, Cost::Fast, &limits, &ctx)
                .is_empty()
        );
    }
}
