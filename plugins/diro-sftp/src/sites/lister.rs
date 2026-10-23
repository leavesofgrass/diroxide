//! The Sites list and the places the plugin offers (R24 sections 3.3 and
//! 3.6): saved sites first, by group and name, then the hosts of the
//! owner's ssh configuration as rows with nothing saved.
//!
//! Every row puts the name first and the state right after it, a word,
//! never a color or a symbol; the user and host come last because they
//! are the longest part: "build, saved, alice@build.example", "build,
//! connected, alice@build.example", "nas, from ssh config". The list's
//! title carries the counts: "Sites, 3 saved, 5 from ssh config".
//!
//! The same rows answer `provider/places` for the go-to-place list, as
//! `name`, `place`, `state` and `detail`.

use serde_json::{Value, json};

use super::model::Site;
use super::words;
use crate::plugin::SCHEME;
use crate::ssh::hosts::ConfigHost;

/// The prefix of a saved site's row id.
pub const SITE_ROW: &str = "site:";

/// The prefix of a configuration host's row id.
pub const CONFIG_ROW: &str = "config:";

/// What a row of the Sites list is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowOf {
    /// A saved site.
    Site(Box<Site>),
    /// A host of the ssh configuration, nothing saved.
    Config(String),
}

/// One row of the Sites list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// What it is.
    pub of: RowOf,
    /// Its state word.
    pub state: &'static str,
}

impl Row {
    /// Its id, the lister's `id` value: `site:k3f9`, `config:nas`.
    pub fn id(&self) -> String {
        match &self.of {
            RowOf::Site(site) => format!("{SITE_ROW}{}", site.id),
            RowOf::Config(name) => format!("{CONFIG_ROW}{name}"),
        }
    }

    /// Its spoken name.
    pub fn name(&self) -> &str {
        match &self.of {
            RowOf::Site(site) => &site.name,
            RowOf::Config(name) => name,
        }
    }

    /// Its user and host, read last; none for a configuration host, whose
    /// details ssh holds.
    pub fn detail(&self) -> Option<String> {
        match &self.of {
            RowOf::Site(site) => Some(site.account()),
            RowOf::Config(_) => None,
        }
    }

    /// The row in words: "build, saved, alice@build.example".
    pub fn words(&self) -> String {
        diroxide_plugins::lines::place_row(self.name(), self.state, self.detail().as_deref())
    }

    /// Where Enter goes.
    pub fn place(&self) -> String {
        match &self.of {
            RowOf::Site(site) => site.place(),
            RowOf::Config(name) => format!("{SCHEME}://{name}/"),
        }
    }

    /// The group, for the column read last; empty for none.
    pub fn group(&self) -> &str {
        match &self.of {
            RowOf::Site(site) => &site.group,
            RowOf::Config(_) => "",
        }
    }

    /// The row as the lister shows it: an item row, its id and columns in
    /// `extra`.
    pub fn lister_row(&self) -> Value {
        json!({
            "name": self.words(),
            "kind": "item",
            "extra": {
                "id": self.id(),
                "state": self.state,
                "address": self.detail().unwrap_or_else(|| self.name().to_owned()),
                "group": self.group(),
            },
        })
    }

    /// The row as `provider/places` answers it.
    pub fn place_row(&self) -> Value {
        let mut row = json!({
            "name": self.name(),
            "place": self.place(),
            "state": self.state,
        });
        if let Some(detail) = self.detail() {
            row["detail"] = json!(detail);
        }
        row
    }
}

/// The rows: the sites sorted by group then name, each with the state
/// `state` gives it, then the configuration's hosts in their files'
/// order, without one a saved alias site already names.
pub fn rows(
    sites: &[Site],
    hosts: &[ConfigHost],
    state: impl Fn(&Site) -> &'static str,
) -> Vec<Row> {
    let mut saved: Vec<&Site> = sites.iter().collect();
    saved.sort_by_key(|s| (s.group.to_lowercase(), s.name.to_lowercase()));
    let mut out: Vec<Row> = saved
        .into_iter()
        .map(|site| Row {
            of: RowOf::Site(Box::new(site.clone())),
            state: state(site),
        })
        .collect();
    for host in hosts {
        if sites
            .iter()
            .any(|s| s.alias.as_deref() == Some(host.name.as_str()))
        {
            continue;
        }
        out.push(Row {
            of: RowOf::Config(host.name.clone()),
            state: words::FROM_CONFIG,
        });
    }
    out
}

/// The list's title, its counts first.
pub fn title(rows: &[Row]) -> String {
    let saved = rows
        .iter()
        .filter(|r| matches!(r.of, RowOf::Site(_)))
        .count();
    words::sites_title(saved, rows.len() - saved)
}

/// `provider/places`' answer.
pub fn places(rows: &[Row]) -> Value {
    json!({"places": rows.iter().map(Row::place_row).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::lines::MAX_LINE_CELLS;

    fn host(name: &str) -> ConfigHost {
        ConfigHost {
            name: name.to_owned(),
            file: PathBuf::from("config"),
        }
    }

    fn sites() -> Vec<Site> {
        vec![
            Site {
                user: Some("alice".into()),
                group: "Work".into(),
                ..Site::new("k3f9", "build", "build.example")
            },
            Site {
                alias: Some("nas".into()),
                host: None,
                ..Site::new("n4s0", "nas", "")
            },
            Site {
                user: Some("ada".into()),
                port: Some(2222),
                ..Site::new("w3b1", "web", "web.example")
            },
        ]
    }

    #[test]
    fn the_golden_rows_put_the_state_right_after_the_name() {
        let rows = rows(&sites(), &[host("nas"), host("backup")], |s| {
            if s.id == "w3b1" {
                words::CONNECTED
            } else {
                words::SAVED
            }
        });
        let said: Vec<String> = rows.iter().map(Row::words).collect();
        assert_eq!(
            said,
            [
                "nas, saved, nas",
                "web, connected, ada@web.example:2222",
                "build, saved, alice@build.example",
                "backup, from ssh config",
            ]
        );
        assert_eq!(title(&rows), "Sites, 3 saved, 1 from ssh config");
        for line in &said {
            assert!(line.chars().count() <= MAX_LINE_CELLS, "{line}");
        }
        assert_eq!(rows[2].id(), "site:k3f9");
        assert_eq!(rows[3].id(), "config:backup");
        assert_eq!(rows[3].place(), "sftp://backup/");
        assert_eq!(rows[2].lister_row()["extra"]["group"], "Work");
        assert_eq!(rows[2].lister_row()["kind"], "item");
    }

    #[test]
    fn places_answer_with_a_state_word_and_no_secret() {
        let rows = rows(&sites(), &[host("backup")], |_| words::LOST);
        let answer = places(&rows);
        assert_eq!(
            answer["places"][2],
            json!({
                "name": "build",
                "place": "sftp://alice@build.example/",
                "state": "lost",
                "detail": "alice@build.example",
            })
        );
        assert_eq!(
            answer["places"][3],
            json!({"name": "backup", "place": "sftp://backup/", "state": "from ssh config"})
        );
        assert!(!answer.to_string().contains("secret"));
    }
}
