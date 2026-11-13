//! The runtime half of batch rename (alpha 5, lane RA): the folders'
//! facts read on a worker, the editor buffer, the presets and the last
//! rules used, answered as [`crate::rename_batch::RenameNote`].
//!
//! Folders are listed on a worker, since a folder can be slow or remote.
//! The buffer and the presets are small files in diroxide's own folders,
//! written whole through a temporary file (`write_atomic`) on the runtime's
//! thread, as the labels and bookmarks are.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_config::{Paths, write_atomic};
use diroxide_core::Place;
use diroxide_ops::rename_rules::{FolderFacts, Presets, Recipe, RenameSource};
use diroxide_vfs::{OpContext, Registry};

use super::Runtime;
use crate::app::AppEvent;
use crate::rename_batch::{LAST_FILE, PRESETS_FILE, RenameAsk, RenameNote, buffer_name};

/// The facts of each source's folder, in the order first met: the names
/// there now, whether case matters, the place's name rules. A folder that
/// cannot be read gets [`FolderFacts::unknown`], the stricter guess.
pub(crate) fn folder_facts(
    registry: &Registry,
    sources: &[RenameSource],
) -> Vec<(Place, FolderFacts)> {
    let ctx = OpContext::new();
    let mut seen: HashSet<Place> = HashSet::new();
    let mut facts = Vec::new();
    for source in sources {
        let Some(folder) = source.place.parent() else {
            continue;
        };
        if !seen.insert(folder.clone()) {
            continue;
        }
        let read = registry.resolve(&folder).ok().and_then(|provider| {
            let capabilities = provider.capabilities(&folder);
            provider.list(&folder, &ctx).ok().map(|entries| {
                FolderFacts::from_capabilities(
                    entries.into_iter().map(|entry| entry.name).collect(),
                    &capabilities,
                )
            })
        });
        facts.push((folder, read.unwrap_or_else(FolderFacts::unknown)));
    }
    facts
}

/// Each source read again through its provider; one that cannot be read
/// is marked gone, so its row says "source is gone".
fn fresh_sources(registry: &Registry, sources: &[RenameSource]) -> Vec<RenameSource> {
    let ctx = OpContext::new();
    sources
        .iter()
        .map(|source| {
            registry
                .resolve(&source.place)
                .and_then(|provider| provider.stat(&source.place, &ctx))
                .map(|entry| RenameSource::new(source.place.clone(), entry))
                .unwrap_or_else(|_| source.clone().gone())
        })
        .collect()
}

/// The presets file's text read, or why not; no file is no presets.
fn read_presets(paths: Option<&Paths>) -> Result<Presets, String> {
    let Some(paths) = paths else {
        return Ok(Presets::default());
    };
    match std::fs::read_to_string(paths.config_dir.join(PRESETS_FILE)) {
        Ok(text) => Presets::from_toml(&text).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Presets::default()),
        Err(error) => Err(format!("Rename presets not read: {}", error_words(&error))),
    }
}

/// The last rules used, when they can be read.
fn read_last(paths: Option<&Paths>) -> Option<Recipe> {
    let text = std::fs::read_to_string(paths?.state_dir.join(LAST_FILE)).ok()?;
    Recipe::from_toml(&text).ok()
}

/// An I/O error in lowercase words.
fn error_words(error: &std::io::Error) -> String {
    let text = error.to_string();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => text,
    }
}

/// The buffer's path for a batch, in the state folder.
fn buffer_path(paths: &Paths, batch: u64) -> PathBuf {
    paths.state_dir.join(buffer_name(batch))
}

/// Writes a buffer and gives its place.
fn write_buffer(paths: Option<&Paths>, batch: u64, text: &str) -> Result<Place, String> {
    let paths = paths.ok_or_else(|| "diroxide has no state folder".to_owned())?;
    let path = buffer_path(paths, batch);
    write_atomic(&path, text).map_err(|error| error_words(&error))?;
    place_of(&path)
}

/// The place of a local path.
fn place_of(path: &Path) -> Result<Place, String> {
    let text = path
        .to_str()
        .ok_or_else(|| "the state folder's name is not valid Unicode".to_owned())?;
    Place::parse(text).map_err(|error| error.to_string())
}

impl Runtime {
    /// Carries out one ask of lane RA.
    pub(super) fn rename_request(&mut self, ask: RenameAsk) {
        match ask {
            RenameAsk::Open { batch, sources } => {
                let registry = Arc::clone(&self.registry);
                let presets = read_presets(self.paths.as_ref());
                let last = read_last(self.paths.as_ref());
                self.worker(move |shared| {
                    let facts = folder_facts(&registry, &sources);
                    shared.send(AppEvent::Renaming(Box::new(RenameNote::Opened {
                        batch,
                        facts,
                        presets,
                        last,
                    })));
                });
            }
            RenameAsk::Recheck { batch, sources } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let sources = fresh_sources(&registry, &sources);
                    let facts = folder_facts(&registry, &sources);
                    shared.send(AppEvent::Renaming(Box::new(RenameNote::Rechecked {
                        batch,
                        sources,
                        facts,
                    })));
                });
            }
            RenameAsk::WriteBuffer { batch, text } => {
                let result = write_buffer(self.paths.as_ref(), batch, &text);
                self.queue
                    .push_back(AppEvent::Renaming(Box::new(RenameNote::BufferWritten {
                        batch,
                        result,
                    })));
            }
            RenameAsk::ReadBuffer { batch, place } => {
                let result = match place.local_path() {
                    Some(path) => {
                        std::fs::read_to_string(path).map_err(|error| error_words(&error))
                    }
                    None => Err("the buffer is not on this computer".to_owned()),
                };
                self.queue
                    .push_back(AppEvent::Renaming(Box::new(RenameNote::BufferRead {
                        batch,
                        result,
                    })));
            }
            RenameAsk::DropBuffer { place } => {
                // Only a buffer this lane made: a file named as a buffer,
                // directly in the state folder.
                if let (Some(paths), Some(path)) = (&self.paths, place.local_path()) {
                    let path = Path::new(path);
                    let ours =
                        path.parent() == Some(paths.state_dir.as_path())
                            && path.file_name().and_then(|name| name.to_str()).is_some_and(
                                |name| name.starts_with("rename-") && name.ends_with(".txt"),
                            );
                    if ours {
                        // A buffer already gone needs nothing more.
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
            RenameAsk::SavePresets(presets) => {
                let Some(paths) = &self.paths else {
                    return;
                };
                let written = presets
                    .to_toml()
                    .map_err(|error| error.to_string())
                    .and_then(|text| {
                        write_atomic(&paths.config_dir.join(PRESETS_FILE), &text)
                            .map_err(|error| error_words(&error))
                    });
                if let Err(cause) = written {
                    self.queue.push_back(AppEvent::Renaming(Box::new(
                        RenameNote::PresetsNotSaved(cause),
                    )));
                }
            }
            RenameAsk::SaveLast(recipe) => {
                if let Some(paths) = &self.paths
                    && let Ok(text) = recipe.to_toml()
                {
                    // The last rules are a convenience: a failed write
                    // costs only the next batch's starting rules.
                    let _ = write_atomic(&paths.state_dir.join(LAST_FILE), &text);
                }
            }
        }
    }
}
