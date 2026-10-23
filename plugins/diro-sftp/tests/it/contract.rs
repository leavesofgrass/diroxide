//! The provider contract list through the real host (R22 section 8.3;
//! R25 commit 61's test): the host's own `PluginProvider` drives the
//! plugin's program, which starts the fake ssh and speaks to the fake
//! server, as the app will. Listing, details, reading, writing that
//! commits or refuses, folders, renames that never replace unless asked,
//! removes, permissions, and no trash.

use std::io::{Read, Write};
use std::sync::Arc;

use diroxide_config::PluginSettings;
use diroxide_core::{Attributes, EntryKind, Place};
use diroxide_plugins::host::QUIT_BUDGET;
use diroxide_plugins::provider::PluginProvider;
use diroxide_plugins::{Host, Manifest, PluginInfo};
use diroxide_vfs::{AttributeChange, OpContext, Provider, VfsError, WriteMode};
use serde_json::json;

use crate::connect::G_OUTPUT;
use crate::fakes::fake_ssh_in;
use crate::{plugin_exe, plugin_folder};

struct Fixture {
    _root: tempfile::TempDir,
    host: Arc<Host>,
    provider: PluginProvider,
}

fn fixture(files: serde_json::Value, dirs: serde_json::Value) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"g_output": G_OUTPUT, "script": [{"serve": {"files": files, "dirs": dirs}}]}),
    );
    let folder = plugin_folder(root.path(), &plugin_exe());
    let manifest =
        Manifest::from_toml(&std::fs::read_to_string(folder.join("plugin.toml")).unwrap()).unwrap();
    let mut host = Host::new(&PluginSettings::default())
        .with_state_dir(root.path().join("state"))
        .with_registry(Arc::new(diroxide_vfs::Registry::with_local()));
    host.add(PluginInfo {
        manifest,
        folder,
        enabled: true,
    });
    // One session: each start of the fake ssh is a fake server of its own.
    let serde_json::Value::Object(values) = json!({"ssh_program": ssh, "sessions": 1}) else {
        unreachable!()
    };
    host.set_settings("sftp", values);
    let host = Arc::new(host);
    let provider = PluginProvider::new(&host, "sftp", "sftp").unwrap();
    Fixture {
        _root: root,
        host,
        provider,
    }
}

fn p(text: &str) -> Place {
    Place::parse(text).unwrap()
}

fn read_text(provider: &PluginProvider, place: &Place) -> String {
    let mut text = String::new();
    provider
        .read(place, &OpContext::new())
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn write_text(
    provider: &PluginProvider,
    place: &Place,
    mode: WriteMode,
    text: &str,
) -> Result<(), VfsError> {
    let mut writer = provider.write(place, mode, &OpContext::new())?;
    writer.write_all(text.as_bytes()).unwrap();
    writer.commit(true)
}

#[test]
fn the_provider_contract_list_holds_through_the_host() {
    let f = fixture(
        json!({"/home/ada/a.txt": "alpha", "/home/ada/old.txt": "old"}),
        json!(["/home/ada/docs"]),
    );
    let ctx = OpContext::new();
    let provider = &f.provider;
    let home = p("sftp://build.example/home/ada");

    // Listing and details.
    let mut names: Vec<String> = provider
        .list(&home, &ctx)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "docs", "old.txt"]);
    let a = p("sftp://build.example/home/ada/a.txt");
    assert_eq!(provider.stat(&a, &ctx).unwrap().size, Some(5));
    assert!(matches!(
        provider.stat(&p("sftp://build.example/home/ada/none"), &ctx),
        Err(VfsError::NotFound)
    ));

    // Reading, whole and ranged.
    assert_eq!(read_text(provider, &a), "alpha");

    // Writing: a new file commits; a taken name is refused unless
    // overwriting.
    let b = p("sftp://build.example/home/ada/docs/b.txt");
    write_text(provider, &b, WriteMode::CreateNew, "bravo").unwrap();
    assert_eq!(read_text(provider, &b), "bravo");
    assert!(matches!(
        write_text(provider, &b, WriteMode::CreateNew, "again"),
        Err(VfsError::AlreadyExists)
    ));
    write_text(provider, &b, WriteMode::Overwrite, "bravo two").unwrap();
    assert_eq!(read_text(provider, &b), "bravo two");
    let listed: Vec<String> = provider
        .list(&p("sftp://build.example/home/ada/docs"), &ctx)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(listed, ["b.txt"], "no unfinished file is shown");

    // Folders.
    let new = p("sftp://build.example/home/ada/new");
    provider.create_dir(&new, &ctx).unwrap();
    assert_eq!(provider.stat(&new, &ctx).unwrap().kind, EntryKind::Folder);

    // Renames never replace, unless asked; a replacing rename uses
    // posix-rename.
    let c = p("sftp://build.example/home/ada/c.txt");
    provider.rename(&a, &c, &ctx).unwrap();
    assert!(matches!(provider.stat(&a, &ctx), Err(VfsError::NotFound)));
    let old = p("sftp://build.example/home/ada/old.txt");
    assert!(matches!(
        provider.rename(&c, &old, &ctx),
        Err(VfsError::AlreadyExists)
    ));
    assert!(provider.capabilities(&old).replace_rename);
    provider.rename_replacing(&c, &old, &ctx).unwrap();
    assert_eq!(read_text(provider, &old), "alpha");

    // Permissions.
    let undo = provider
        .set_attributes(
            &old,
            &AttributeChange {
                attributes: Some(Attributes::Unix(0o600)),
                ..AttributeChange::default()
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(undo.attributes, Some(Attributes::Unix(0o644)));
    assert_eq!(
        provider.stat(&old, &ctx).unwrap().attributes,
        Some(Attributes::Unix(0o600))
    );

    // Free space for the copy plan's room check, from statvfs; the fake
    // server has no copy-data, so copies stream.
    let space = provider.space(&home, &ctx).unwrap();
    assert!(space.free_to_user.is_some_and(|free| free > 0));
    assert!(space.total.is_some());
    assert_eq!(
        provider.copy_hint(&old, &p("sftp://build.example/home/ada/copy.txt")),
        diroxide_vfs::CopyHint::Stream
    );

    // Removes are permanent: there is no trash to fall back on.
    let caps = provider.capabilities(&old);
    assert!(caps.writable);
    assert!(!caps.trash);
    assert!(provider.trash(&old, &ctx).is_err());
    provider.remove(&old, &ctx).unwrap();
    assert!(matches!(provider.stat(&old, &ctx), Err(VfsError::NotFound)));
    provider.remove(&new, &ctx).unwrap();
    assert!(matches!(provider.stat(&new, &ctx), Err(VfsError::NotFound)));
    f.host.shutdown_all(QUIT_BUDGET);
}
