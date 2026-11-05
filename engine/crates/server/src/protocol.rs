//! That `engine/PROTOCOL.md` says what the engine does (plan of phase 3, T17): every operation and
//! edit that the model takes is in its tables, with every field that it takes; every reason that
//! a refusal names is in it; and every method that the engine answers by its name is listed as a
//! method, or a notification. That every method that it lists is answered is checked by the
//! engine itself, in `tests/rpc/protocol.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use columbina_model::{Edit, Op};
use serde_json::json;

/// The directory of this crate.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The text of `PROTOCOL.md`.
fn protocol() -> String {
    std::fs::read_to_string(crate_dir().join("../../PROTOCOL.md")).expect("PROTOCOL.md is there")
}

/// The cells of a row of a table of Markdown, trimmed; none for a line that is not a row. A `|`
/// written `\|` is kept in its cell.
fn cells(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    let mut cells = vec![String::new()];
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cells.last_mut().expect("a cell").push('|');
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().expect("a cell").push(c),
        }
    }
    Some(
        cells
            .into_iter()
            .map(|cell| cell.trim().to_owned())
            .collect(),
    )
}

/// Every table of the text: its header, and its rows below the line under it.
fn tables(text: &str) -> Vec<(Vec<String>, Vec<Vec<String>>)> {
    let mut tables = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(header) = cells(line) else {
            continue;
        };
        if !lines
            .peek()
            .is_some_and(|next| next.trim_start().starts_with("|---"))
        {
            continue;
        }
        lines.next();
        let mut rows = Vec::new();
        while let Some(row) = lines.peek().and_then(|next| cells(next)) {
            rows.push(row);
            lines.next();
        }
        tables.push((header, rows));
    }
    tables
}

/// What is written between backticks in a cell.
fn quoted(cell: &str) -> Vec<&str> {
    cell.split('`').skip(1).step_by(2).collect()
}

/// The names in the first column of the tables whose first header is `first` and that have
/// `columns` columns (any number where none), each with its row.
fn named_rows(text: &str, first: &str, columns: Option<usize>) -> BTreeMap<String, Vec<String>> {
    let mut named = BTreeMap::new();
    for (header, rows) in tables(text) {
        if header[0] != first || columns.is_some_and(|n| header.len() != n) {
            continue;
        }
        let mut before: Option<Vec<String>> = None;
        for mut row in rows {
            // A row that says it takes the same as the one above it.
            if row.get(1).is_some_and(|cell| cell == "same") {
                if let Some(above) = &before {
                    row[1].clone_from(&above[1]);
                }
            }
            for name in quoted(&row[0]) {
                named.insert(name.to_owned(), row.clone());
            }
            before = Some(row);
        }
    }
    named
}

/// The methods of the tables of methods (`| method | params | result |`).
pub fn methods(text: &str) -> BTreeSet<String> {
    named_rows(text, "method", Some(3))
        .into_keys()
        .filter(|name| !name.ends_with(".*"))
        .collect()
}

/// The notifications of the table of notifications (`| method | params |`).
pub fn notifications(text: &str) -> BTreeSet<String> {
    named_rows(text, "method", Some(2)).into_keys().collect()
}

/// The names that a message of serde says it expected: ``expected one of `a`, `b`, `c` ``,
/// ``expected `a` or `b` ``, or ``expected `a` ``.
fn expected(message: &str) -> Vec<String> {
    let Some((_, after)) = message.split_once("expected ") else {
        return vec![];
    };
    let after = after.strip_prefix("one of ").unwrap_or(after);
    after
        .split(", ")
        .flat_map(|names| names.split(" or "))
        .map_while(|name| {
            let name = name.strip_prefix('`')?;
            let end = name.find('`')?;
            Some(name[..end].to_owned())
        })
        .collect()
}

/// The names of the variants of an enum tagged by `tag`, and the fields of each, as serde reads
/// them: a variant that is not one, and a field that is not one, are refused with the names
/// that it takes.
fn variants<T: serde::de::DeserializeOwned>(tag: &str) -> BTreeMap<String, Vec<String>> {
    let not_one = json!({ tag: "\u{1}" });
    let Err(error) = serde_json::from_value::<T>(not_one) else {
        panic!("no {tag} is named so");
    };
    expected(&error.to_string())
        .into_iter()
        .map(|name| {
            let not_a_field = json!({ tag: name, "\u{1}": 0 });
            let fields = match serde_json::from_value::<T>(not_a_field) {
                Ok(_) => vec![],
                Err(error) => expected(&error.to_string()),
            };
            (name, fields)
        })
        .collect()
}

/// Whether a cell names a field: as `` `field` ``, `` `field?` ``, or at the start of a quote
/// that says more of it (`` `clips: [id, ...]` ``, `` `action: add\|remove` ``).
fn names_field(cell: &str, field: &str) -> bool {
    quoted(cell).iter().any(|quote| {
        quote.strip_prefix(field).is_some_and(|rest| {
            rest.is_empty()
                || rest.starts_with('?')
                || rest.starts_with(':')
                || rest.starts_with(' ')
        })
    })
}

/// The reasons of the refusals of operations: the variants of `Fields` in `ops.rs`.
fn reasons_of_operations() -> BTreeSet<String> {
    let source = std::fs::read_to_string(crate_dir().join("../model/src/ops.rs")).unwrap();
    let start = source
        .find("enum Fields {")
        .expect("the reasons of operations");
    let body = &source[start..];
    let body = &body[..body.find("\n}").expect("the end of Fields")];
    body.lines()
        .filter_map(|line| {
            let name = line.strip_prefix("    ")?;
            let first = name.chars().next()?;
            if !first.is_ascii_uppercase() {
                return None;
            }
            let end = name.find([' ', ',', '{']).unwrap_or(name.len());
            Some(snake(&name[..end]))
        })
        .collect()
}

/// The reasons of the refusals of edits and transactions: `"reason": "…"` as they are written.
fn reasons_written(file: &Path) -> BTreeSet<String> {
    let source = std::fs::read_to_string(file).unwrap();
    source
        .split("\"reason\": \"")
        .skip(1)
        .filter_map(|rest| Some(rest[..rest.find('"')?].to_owned()))
        .collect()
}

/// `SnakeCase` as `snake_case`.
fn snake(name: &str) -> String {
    let mut snake = String::new();
    for (at, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && at > 0 {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    snake
}

/// The methods that the engine answers by name: the strings of the patterns of the arms of a
/// `match` (`"a.b" =>`, `"a.b" | "a.c" =>`, `| "a.d"` on a line of its own) in its sources.
fn methods_matched() -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let mut dirs = vec![crate_dir().join("src")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("protocol.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for line in source.lines() {
                let mut rest = line.trim_start().trim_start_matches('|').trim_start();
                let mut names = Vec::new();
                while let Some(after) = rest.strip_prefix('"') {
                    let Some(end) = after.find('"') else {
                        break;
                    };
                    names.push(&after[..end]);
                    rest = after[end + 1..].trim_start();
                    rest = rest.strip_prefix('|').map_or(rest, str::trim_start);
                }
                let arm = rest.starts_with("=>") || rest.starts_with("if ") || rest.is_empty();
                if !arm || names.is_empty() {
                    continue;
                }
                for name in names {
                    let dotted = name.contains('.')
                        && name.split('.').all(|part| {
                            !part.is_empty()
                                && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                        });
                    if dotted {
                        found.insert(name.to_owned(), path.display().to_string());
                    }
                }
            }
        }
    }
    found
}

/// Dotted names in the arms of a `match` that are no methods, and the files they are in.
const NOT_METHODS: [(&str, &str); 2] =
    [("a.cube", "viewer_preview.rs"), ("ffmpeg.exe", "proxy.rs")];

/// The old names that edits are read by.
const OLD_NAMES: [&str; 1] = ["append_grade"];

#[test]
fn every_operation_is_in_a_table_of_operations_with_every_field_that_it_takes() {
    let text = protocol();
    let written = named_rows(&text, "op", None);
    let ops = variants::<Op>("op");
    assert!(ops.len() > 50, "{ops:?}");
    let mut wrong = Vec::new();
    for (op, fields) in &ops {
        let Some(row) = written.get(op) else {
            wrong.push(format!("`{op}` is in no table of operations"));
            continue;
        };
        for field in fields {
            if !names_field(&row[1], field) {
                wrong.push(format!(
                    "`{op}` takes `{field}`, which its row does not name"
                ));
            }
        }
    }
    for op in written.keys() {
        if !ops.contains_key(op) {
            wrong.push(format!("`{op}` is written as an operation, and is none"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_edit_is_in_a_table_of_edits_with_every_field_that_it_takes() {
    let text = protocol();
    let written = named_rows(&text, "edit", None);
    let mut edits = variants::<Edit>("edit");
    assert!(edits.len() > 40, "{edits:?}");
    // An old name that an edit is read by, which the protocol says it is.
    for old in OLD_NAMES {
        assert!(edits.remove(old).is_some(), "`{old}` is read");
        assert!(
            text.contains(&format!("`{old}`, its old name")),
            "`{old}` is written"
        );
    }
    let mut wrong = Vec::new();
    for (edit, fields) in &edits {
        let Some(row) = written.get(edit) else {
            wrong.push(format!("`{edit}` is in no table of edits"));
            continue;
        };
        // The tables of the edits of timelines name every field besides `timeline`.
        for field in fields.iter().filter(|field| *field != "timeline") {
            if !names_field(&row[1], field) {
                wrong.push(format!(
                    "`{edit}` takes `{field}`, which its row does not name"
                ));
            }
        }
    }
    for edit in written.keys() {
        if !edits.contains_key(edit) {
            wrong.push(format!("`{edit}` is written as an edit, and is none"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_fields_of_an_operation_are_read_from_what_serde_says() {
    let ops = variants::<Op>("op");
    assert_eq!(ops["set_proxy"], ["media", "proxy"]);
    assert_eq!(ops["set_media_streams"], ["id", "layout", "streams"]);
    assert_eq!(ops["remove_media"], ["id"]);
    assert_eq!(ops["remove_bus"], ["timeline", "id"]);
    assert_eq!(expected("unknown field `x`, expected `a`"), ["a"]);
    let edits = variants::<Edit>("edit");
    assert!(edits["lane_points"].contains(&"of".to_owned()));
    assert!(names_field("`timeline`, `clips: [id, ...]`, `by`", "clips"));
    assert!(names_field("`index?`", "index"));
    assert!(!names_field("`indexes`", "index"));
    assert_eq!(snake("OutOfRange"), "out_of_range");
}

#[test]
fn every_reason_of_a_refusal_is_in_the_protocol() {
    let text = protocol();
    let quoted: BTreeSet<&str> = text.lines().flat_map(quoted).collect();
    let mut reasons = reasons_of_operations();
    assert!(reasons.contains("audio_settings") && reasons.contains("missing"));
    let model = crate_dir().join("../model/src");
    reasons.extend(reasons_written(&model.join("edit/mod.rs")));
    reasons.extend(reasons_written(&model.join("history.rs")));
    assert!(reasons.contains("ripple_blocked"), "{reasons:?}");
    let missing: Vec<_> = reasons
        .iter()
        .filter(|reason| !quoted.contains(reason.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "refusals not in PROTOCOL.md: {missing:?}"
    );
}

/// The notifications that the engine sends by name: the first argument of
/// `rpc::notification_line`, where it is written there.
fn notifications_sent() -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let mut dirs = vec![crate_dir().join("src")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("protocol.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for (_, after) in source
                .match_indices("notification_line(")
                .map(|(at, call)| (at, source[at + call.len()..].trim_start()))
            {
                let Some(name) = after.strip_prefix('"') else {
                    continue;
                };
                let name = &name[..name.find('"').unwrap_or(0)];
                if name.contains('.') {
                    found.insert(name.to_owned(), path.display().to_string());
                }
            }
        }
    }
    found
}

#[test]
fn every_notification_that_the_engine_sends_by_name_is_in_the_protocol() {
    let notifications = notifications(&protocol());
    let sent = notifications_sent();
    assert!(sent.contains_key("automation.written"), "{sent:?}");
    let missing: Vec<_> = sent
        .iter()
        .filter(|(name, _)| !notifications.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "sent by the engine, not in the table: {missing:?}"
    );
}

#[test]
fn every_method_that_the_engine_answers_by_name_is_in_the_protocol() {
    let text = protocol();
    let methods = methods(&text);
    let notifications = notifications(&text);
    assert!(methods.contains("transaction.apply") && methods.contains("transport.seek"));
    assert!(notifications.contains("project.changed"));
    let matched = methods_matched();
    assert!(matched.contains_key("automation.punch"), "{matched:?}");
    // Every dotted name of an arm is a method, of a namespace known or new (`meters.*`), but
    // these, which are names of files that a match tells apart.
    for (name, file) in NOT_METHODS {
        assert!(
            matched.get(name).is_some_and(|found| found.ends_with(file)),
            "{name} is no longer in {file}: take it off NOT_METHODS"
        );
    }
    let missing: Vec<_> = matched
        .iter()
        .filter(|(name, _)| !NOT_METHODS.iter().any(|(not, _)| not == name))
        .filter(|(name, _)| !methods.contains(*name) && !notifications.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "named by the engine, not in PROTOCOL.md: {missing:?}"
    );
}
