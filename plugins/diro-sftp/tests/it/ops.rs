//! mkdir, rmdir, remove, rename, set attributes and links (R25 commit
//! 61) against the fake server: folders with and without their parents,
//! a remove for good with no trash, renames that never replace unless
//! asked and never replace a folder, permissions said in words, and
//! SYMLINK with the target first.

use diro_sftp::sftp::fake::{Faults, Node};
use serde_json::json;

use crate::connect::{connect, fake_host};
use crate::host::kind;

#[test]
fn folders_are_made_with_or_without_their_parents() {
    let (host, opener, _root) = fake_host(Faults::default());
    connect(&host, "sftp://build.example/").unwrap();
    host.call(
        "provider/create_dir",
        json!({"place": "sftp://build.example/home/ada/new", "parents": false}),
    )
    .unwrap();
    let missing = host
        .call(
            "provider/create_dir",
            json!({"place": "sftp://build.example/home/ada/x/y/z", "parents": false}),
        )
        .unwrap_err();
    assert_eq!(kind(&missing), "not_found");
    host.call(
        "provider/create_dir",
        json!({"place": "sftp://build.example/home/ada/x/y/z", "parents": true}),
    )
    .unwrap();
    let fs = opener.fs().lock().unwrap();
    assert_eq!(fs.node(b"/home/ada/new"), Some(&Node::Dir));
    assert_eq!(fs.node(b"/home/ada/x/y/z"), Some(&Node::Dir));
}

#[test]
fn removes_are_for_good_and_there_is_no_trash() {
    let (host, opener, _root) = fake_host(Faults::default());
    {
        let mut fs = opener.fs().lock().unwrap();
        fs.add_file(b"/srv/a.txt", b"a");
        fs.add_dir(b"/srv/empty");
        fs.add_link(b"/srv/l", b"a.txt");
    }
    connect(&host, "sftp://build.example/").unwrap();
    let trash = host
        .call(
            "provider/trash",
            json!({"place": "sftp://build.example/srv/a.txt"}),
        )
        .unwrap_err();
    assert_eq!(kind(&trash), "unsupported");
    assert_eq!(trash["message"], "SFTP has no trash.");
    for place in ["a.txt", "empty", "l"] {
        host.call(
            "provider/remove",
            json!({"place": format!("sftp://build.example/srv/{place}"), "recursive": false}),
        )
        .unwrap();
    }
    let fs = opener.fs().lock().unwrap();
    assert!(fs.node(b"/srv/a.txt").is_none());
    assert!(fs.node(b"/srv/empty").is_none());
    assert!(fs.node(b"/srv/l").is_none());
    drop(fs);
    let gone = host
        .call(
            "provider/remove",
            json!({"place": "sftp://build.example/srv/a.txt", "recursive": false}),
        )
        .unwrap_err();
    assert_eq!(kind(&gone), "not_found");
    let recursive = host
        .call(
            "provider/remove",
            json!({"place": "sftp://build.example/srv", "recursive": true}),
        )
        .unwrap_err();
    assert_eq!(kind(&recursive), "unsupported");
}

#[test]
fn renames_never_replace_unless_asked_and_never_a_folder() {
    for faults in [
        Faults::default(),
        Faults {
            no_posix_rename: true,
            ..Faults::default()
        },
    ] {
        let (host, opener, _root) = fake_host(faults.clone());
        {
            let mut fs = opener.fs().lock().unwrap();
            fs.add_file(b"/srv/a.txt", b"new");
            fs.add_file(b"/srv/b.txt", b"old");
            fs.add_dir(b"/srv/dir");
        }
        connect(&host, "sftp://build.example/").unwrap();
        let rename = |from: &str, to: &str, overwrite: bool| {
            host.call(
                "provider/rename",
                json!({"from": format!("sftp://build.example/srv/{from}"),
                    "to": format!("sftp://build.example/srv/{to}"), "overwrite": overwrite}),
            )
        };
        assert_eq!(
            kind(&rename("a.txt", "b.txt", false).unwrap_err()),
            "already_exists"
        );
        assert_eq!(
            kind(&rename("a.txt", "dir", true).unwrap_err()),
            "already_exists"
        );
        assert_eq!(
            kind(&rename("dir", "b.txt", true).unwrap_err()),
            "already_exists"
        );
        let replaced = rename("a.txt", "b.txt", true);
        if faults.no_posix_rename {
            let refused = replaced.unwrap_err();
            assert_eq!(kind(&refused), "unsupported");
            assert_eq!(refused["message"], "This server cannot rename over a file.");
            assert_eq!(
                opener.fs().lock().unwrap().file(b"/srv/b.txt"),
                Some(&b"old"[..])
            );
        } else {
            replaced.unwrap();
            assert_eq!(
                opener.fs().lock().unwrap().file(b"/srv/b.txt"),
                Some(&b"new"[..])
            );
        }
        rename("dir", "moved", false).unwrap();
        assert_eq!(
            opener.fs().lock().unwrap().node(b"/srv/moved"),
            Some(&Node::Dir)
        );
        let elsewhere = host
            .call(
                "provider/rename",
                json!({"from": "sftp://build.example/srv/moved",
                    "to": "sftp://other.example/srv/moved", "overwrite": false}),
            )
            .unwrap_err();
        assert_eq!(kind(&elsewhere), "unsupported");
    }
}

#[test]
fn permissions_and_times_are_set_and_said_in_words() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.fs().lock().unwrap().add_file(b"/srv/a.txt", b"a");
    connect(&host, "sftp://build.example/").unwrap();
    let answer = host
        .call(
            "provider/set_attributes",
            json!({"place": "sftp://build.example/srv/a.txt", "mode": 0o600,
                "modified": "2023-11-14T22:13:20Z"}),
        )
        .unwrap();
    assert_eq!(answer["words"], "Permissions now 600.");
    assert_eq!(answer["mode"], 0o600);
    assert_eq!(answer["modified_seconds"], 1_700_000_000);
    let stat = host
        .call(
            "provider/stat",
            json!({"place": "sftp://build.example/srv/a.txt"}),
        )
        .unwrap();
    assert_eq!(stat["mode"], 0o600);
    assert_eq!(stat["modified"], "2023-11-14T22:13:20Z");
    let too_old = host
        .call(
            "provider/set_attributes",
            json!({"place": "sftp://build.example/srv/a.txt", "modified": "1900-01-01T00:00:00Z"}),
        )
        .unwrap_err();
    assert_eq!(too_old["message"], "SFTP keeps dates from 1970 to 2106.");
}

#[test]
fn symlink_sends_the_target_first() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.fs().lock().unwrap().add_file(b"/srv/a.txt", b"a");
    connect(&host, "sftp://build.example/").unwrap();
    host.call(
        "provider/create_link",
        json!({"place": "sftp://build.example/srv/latest", "target": "a.txt", "kind": "symbolic"}),
    )
    .unwrap();
    // The fake reads OpenSSH's order: the target, then the link's path.
    assert_eq!(
        opener.fs().lock().unwrap().node(b"/srv/latest"),
        Some(&Node::Link(b"a.txt".to_vec()))
    );
    let stat = host
        .call(
            "provider/stat",
            json!({"place": "sftp://build.example/srv/latest"}),
        )
        .unwrap();
    assert_eq!(stat["link_target"], "a.txt");
    let hard = host
        .call(
            "provider/create_link",
            json!({"place": "sftp://build.example/srv/h", "target": "/srv/a.txt", "kind": "hard"}),
        )
        .unwrap_err();
    assert_eq!(kind(&hard), "unsupported");
}
