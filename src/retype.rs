//! Applying a rename table to a corpus that is already typed.
//!
//! [`crate::migrate`] writes the frontmatter a document has not got. This is
//! its opposite and its complement: every document already carries a `type`,
//! the names it carries are being reduced to a smaller vocabulary, and the
//! only field allowed to move is that one.
//!
//! One bundle in this estate needs it — 672 documents against 39 names the
//! ratified vocabulary reduces to 26 — and that bundle is also the
//! byte-for-byte parity target for the whole port. So three rules hold the
//! pass:
//!
//! * **Nothing but `type`.** One value is spliced over its own byte range.
//!   Quoting, key order, indentation, trailing whitespace and line endings are
//!   the file's, not this tool's. A pass that changes 672 documents is not the
//!   pass to also normalise anything.
//! * **Parse, never grep.** This estate has documents whose `^type:` line is
//!   an exemplar inside a fenced code block in a prompt template, and it is
//!   not the document's own type. `grep -rh '^type:'` finds 68 `Meeting
//!   Summary` lines in the 757-file reference bundle against 67 documents;
//!   the count is how you tell. See [`crate::frontmatter::scalar_span`].
//! * **`type` is never removed.** §11 requires it. A name that is being
//!   retired rather than renamed is a `review` row in the table: its files are
//!   listed and a person retypes or deletes each one. Silently dropping the
//!   one required field would turn a rename into a conformance failure the
//!   pass itself caused.
//!
//! Running twice leaves `git diff --exit-code` clean, structurally: a document
//! whose value already reads as the new name produces identical text and is
//! not a change.

use std::collections::BTreeMap;
use std::path::Path;

use crate::config::{Config, ConfigError, Retype, RetypeTable};
use crate::frontmatter::{ParseError, parse_strict, scalar_span};
use crate::migrate::is_reserved;
use crate::walk;

/// What the table says about one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The table renames it. `rewritten` carries the file's new text, and is
    /// absent when the document already reads as the new name.
    Rename {
        to: String,
        rewritten: Option<String>,
    },
    /// The table refers it to a person, with the reason the row gives.
    Review(String),
    /// The table does not name this type. Left exactly as written.
    Unnamed,
}

/// What `--retype` would do to one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    /// The `type` the document carries today.
    pub from: String,
    pub action: Action,
}

impl Entry {
    /// One TSV row: path, old type, new type, what happens.
    #[must_use]
    pub fn tsv(&self) -> String {
        let (to, what) = match &self.action {
            Action::Rename {
                to,
                rewritten: Some(_),
            } => (to.as_str(), "rename"),
            Action::Rename {
                to,
                rewritten: None,
            } => (to.as_str(), "already-renamed"),
            Action::Review(_) => ("-", "review"),
            Action::Unnamed => ("-", "not-in-table"),
        };
        format!("{}\t{}\t{}\t{}", self.path, self.from, to, what)
    }
}

/// The whole retype pass for a bundle.
#[derive(Debug, Default)]
pub struct Plan {
    pub entries: Vec<Entry>,
    /// Files whose frontmatter does not parse, with the reason. This tool
    /// refuses to splice into a block it could not read.
    pub unparseable: Vec<(String, String)>,
}

impl Plan {
    /// Files this run would rewrite.
    pub fn changes(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| {
            matches!(
                entry.action,
                Action::Rename {
                    rewritten: Some(_),
                    ..
                }
            )
        })
    }

    /// Files a person decides, which are the only judgements in the pass.
    pub fn judgements(&self) -> impl Iterator<Item = (&Entry, &str)> {
        self.entries.iter().filter_map(|entry| match &entry.action {
            Action::Review(why) => Some((entry, why.as_str())),
            _ => None,
        })
    }

    /// Type names present in the bundle that the table does not mention, with
    /// how many documents carry each.
    ///
    /// A name the table forgot looks exactly like a name it deliberately
    /// leaves alone, and the difference is the operator's to see. Thirteen of
    /// the reference bundle's 39 names survive unchanged and belong here;
    /// anything else here is a row somebody has yet to write.
    #[must_use]
    pub fn unnamed_types(&self) -> BTreeMap<&str, usize> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for entry in &self.entries {
            if entry.action == Action::Unnamed {
                let slot = counts.entry(entry.from.as_str()).or_default();
                *slot = slot.saturating_add(1);
            }
        }
        counts
    }
}

/// Work out what a retype would do, writing nothing.
///
/// # Errors
///
/// Fails when the `[[retype]]` table in `okf.toml` is not a table this tool
/// will act on. See [`Config::retype_table`].
pub fn plan(root: &Path, config: &Config) -> Result<Plan, ConfigError> {
    let table = config.retype_table()?;
    let mut plan = Plan::default();
    for path in walk::markdown_files(root, &config.paths.skip_names) {
        let Ok(stripped) = path.strip_prefix(root) else {
            continue;
        };
        let relative = walk::to_posix(stripped);
        // The same gating as a migration: §8 and §9 own the reserved names,
        // and writing into a generator's output would be truncated on its next
        // run.
        if is_reserved(&relative) || config.is_generated(&relative) {
            continue;
        }
        let text = walk::read_lossy(&path);
        if let Some(entry) = plan_one(&relative, &text, &table, &mut plan.unparseable) {
            plan.entries.push(entry);
        }
    }
    plan.entries.sort_by(|a, b| a.path.cmp(&b.path));
    plan.unparseable.sort();
    Ok(plan)
}

/// One document, or `None` when it carries no `type` for a table to act on.
fn plan_one(
    relative: &str,
    text: &str,
    table: &RetypeTable,
    unparseable: &mut Vec<(String, String)>,
) -> Option<Entry> {
    let frontmatter = match parse_strict(text) {
        Ok(frontmatter) => frontmatter,
        // No frontmatter at all: nothing here is this document's type, whatever
        // a `^type:` grep would say about its code fences.
        Err(ParseError::NoFence) if !text.starts_with("---") => return None,
        Err(ParseError::NoFence) => {
            unparseable.push((relative.to_owned(), "unterminated fence".to_owned()));
            return None;
        }
        Err(other) => {
            unparseable.push((relative.to_owned(), other.message()));
            return None;
        }
    };

    let from = frontmatter.get_unquoted("type");
    if from.is_empty() {
        return None;
    }

    let action = match table.get(&from) {
        None => Action::Unnamed,
        Some(Retype::Review(why)) => Action::Review(why.clone()),
        Some(Retype::To(to)) => Action::Rename {
            to: to.clone(),
            rewritten: rewrite(text, to).filter(|new| new != text),
        },
    };
    Some(Entry {
        path: relative.to_owned(),
        from,
        action,
    })
}

/// The file with its `type` value replaced and every other byte kept.
///
/// `None` when the block has no `type` at column zero, which cannot happen for
/// a document that parsed with one and is a refusal rather than a guess if it
/// ever does.
fn rewrite(text: &str, to: &str) -> Option<String> {
    let span = scalar_span(text, "type")?;
    let old = text.get(span.clone())?;
    let mut out = text.to_owned();
    out.replace_range(span, &render_value(old, to));
    Some(out)
}

/// The new name, written the way the old one was.
///
/// A retype changes a name and nothing else, so a value written bare stays
/// bare and one written in quotes keeps the quotes it had. The one exception
/// is a name that could not be read back in the style it inherited, which
/// gains double quotes rather than being written unparseable — no vocabulary
/// name in this estate needs it, and a tool that would have written a broken
/// line if one did is a tool waiting to.
fn render_value(old: &str, new: &str) -> String {
    let quoted_with =
        |quote: char| old.len() >= 2 && old.starts_with(quote) && old.ends_with(quote);
    if quoted_with('\'') && !new.contains('\'') {
        return format!("'{new}'");
    }
    if quoted_with('"') || needs_quoting(new) {
        let escaped = new.replace('\\', "\\\\").replace('"', "\\\"");
        return format!("\"{escaped}\"");
    }
    new.to_owned()
}

/// Whether a value cannot be written bare in the frontmatter subset this crate
/// parses.
fn needs_quoting(value: &str) -> bool {
    value.is_empty()
        || value.trim() != value
        || value.contains([':', '#', '"', '\'', '\n', '\r'])
        || value.starts_with([
            '-', '&', '*', '[', ']', '{', '}', '>', '|', '!', '%', '@', '`',
        ])
}

/// Apply a plan.
///
/// # Errors
///
/// Fails when a file cannot be written.
pub fn apply(root: &Path, plan: &Plan) -> std::io::Result<usize> {
    let mut written: usize = 0;
    for entry in plan.changes() {
        let Action::Rename {
            rewritten: Some(text),
            ..
        } = &entry.action
        else {
            continue;
        };
        std::fs::write(root.join(&entry.path), text)?;
        written = written.saturating_add(1);
    }
    Ok(written)
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "a panicking assertion is the point of a test"
)]
mod tests {
    use super::*;
    use crate::config::{RetypeRule, Vocabulary};

    fn config_with(rows: &[(&str, Option<&str>, Option<&str>)]) -> Config {
        Config {
            vocabulary: Vocabulary {
                extends: vec![
                    "core".to_owned(),
                    "capture".to_owned(),
                    "knowledge".to_owned(),
                ],
                types: Vec::new(),
            },
            retype: rows
                .iter()
                .map(|(from, to, review)| RetypeRule {
                    from: (*from).to_owned(),
                    to: to.map(str::to_owned),
                    review: review.map(str::to_owned),
                })
                .collect(),
            ..Config::default()
        }
    }

    fn table(rows: &[(&str, Option<&str>, Option<&str>)]) -> RetypeTable {
        config_with(rows).retype_table().unwrap()
    }

    fn one(text: &str, rows: &[(&str, Option<&str>, Option<&str>)]) -> Option<Entry> {
        plan_one("a/x.md", text, &table(rows), &mut Vec::new())
    }

    /// The new text of a rename, so a test asserts on bytes rather than
    /// pattern-matching an enum in every case.
    fn renamed(entry: &Entry) -> Option<&str> {
        match &entry.action {
            Action::Rename {
                rewritten: Some(text),
                ..
            } => Some(text.as_str()),
            _ => None,
        }
    }

    #[test]
    fn a_rename_moves_the_value_and_nothing_else() {
        let text = "---\ntype: \"Meeting Summary\"\ntitle:   \"Kept\"\nowner:\n  - name: \"A\"\n---\n\n# Body\n\nProse.\n";
        let entry = one(text, &[("Meeting Summary", Some("Meeting"), None)]).unwrap();
        assert_eq!(
            renamed(&entry).unwrap(),
            "---\ntype: \"Meeting\"\ntitle:   \"Kept\"\nowner:\n  - name: \"A\"\n---\n\n# Body\n\nProse.\n"
        );
    }

    #[test]
    fn a_bare_value_stays_bare_and_a_quoted_one_keeps_its_quotes() {
        let bare = one(
            "---\ntype: Email Thread\n---\n\nBody.\n",
            &[("Email Thread", Some("Correspondence"), None)],
        )
        .unwrap();
        assert!(matches!(
            bare.action,
            Action::Rename { rewritten: Some(ref new), .. } if new == "---\ntype: Correspondence\n---\n\nBody.\n"
        ));

        let single = one(
            "---\ntype: 'Email Thread'\n---\n\nBody.\n",
            &[("Email Thread", Some("Correspondence"), None)],
        )
        .unwrap();
        assert!(matches!(
            single.action,
            Action::Rename { rewritten: Some(ref new), .. } if new == "---\ntype: 'Correspondence'\n---\n\nBody.\n"
        ));
    }

    #[test]
    fn a_second_pass_finds_nothing_to_do() {
        let rows = [("Meeting Summary", Some("Meeting"), None)];
        let first = one("---\ntype: \"Meeting Summary\"\n---\n\nBody.\n", &rows).unwrap();
        let new = renamed(&first).unwrap().to_owned();
        // The name is now `Meeting`, which the table does not mention, so the
        // second pass leaves it alone rather than looking for `Meeting` in the
        // table and renaming again.
        let second = one(&new, &rows).unwrap();
        assert_eq!(second.from, "Meeting");
        assert_eq!(second.action, Action::Unnamed);
    }

    /// The one shape that makes a grep wrong. One knowledge bundle in this
    /// estate had exactly one `type: meeting` and it was this; the 757-file
    /// reference bundle has the same shape at line 102 of a prompt
    /// template.
    #[test]
    fn an_exemplar_in_a_code_fence_is_not_the_documents_type() {
        let template =
            "# Meeting Transcript Prompt\n\nEmit:\n\n```yaml\ntype: Meeting Summary\n```\n";
        assert_eq!(
            one(template, &[("Meeting Summary", Some("Meeting"), None)]),
            None,
            "a document with no frontmatter has no type to rename"
        );
    }

    /// A document whose own type is `Template` and which shows a different
    /// type inside a fence: the fence must not be reached, and the block must.
    #[test]
    fn a_fenced_exemplar_below_real_frontmatter_is_left_alone() {
        let text =
            "---\ntype: Prompt Template\ntitle: T\n---\n\n```yaml\ntype: Meeting Summary\n```\n";
        let entry = one(
            text,
            &[
                ("Prompt Template", Some("Template"), None),
                ("Meeting Summary", Some("Meeting"), None),
            ],
        )
        .unwrap();
        assert_eq!(
            renamed(&entry).unwrap(),
            "---\ntype: Template\ntitle: T\n---\n\n```yaml\ntype: Meeting Summary\n```\n"
        );
    }

    #[test]
    fn a_retired_name_is_listed_and_its_type_is_never_removed() {
        let text = "---\ntype: \"Toolkit Index\"\ntitle: \"Index\"\n---\n\nBody.\n";
        let entry = one(
            text,
            &[(
                "Toolkit Index",
                None,
                Some("a hand-maintained listing that §8's generated index.md replaces"),
            )],
        )
        .unwrap();
        assert!(matches!(entry.action, Action::Review(_)));
        assert_eq!(
            entry.action,
            Action::Review(
                "a hand-maintained listing that §8's generated index.md replaces".to_owned()
            )
        );
    }

    #[test]
    fn a_name_the_table_does_not_mention_is_left_alone() {
        let entry = one(
            "---\ntype: System\n---\n\nBody.\n",
            &[("Meeting Summary", Some("Meeting"), None)],
        )
        .unwrap();
        assert_eq!(entry.action, Action::Unnamed);
    }

    #[test]
    fn a_broken_block_is_reported_rather_than_spliced_into() {
        let mut unparseable = Vec::new();
        let entry = plan_one(
            "a/x.md",
            "---\ntype: A\ntype: B\n---\n\nBody.\n",
            &table(&[("A", Some("Meeting"), None)]),
            &mut unparseable,
        );
        assert_eq!(entry, None);
        assert_eq!(unparseable.len(), 1);
    }

    #[test]
    fn a_table_that_would_invent_a_name_is_refused() {
        let err = config_with(&[("Email Thread", Some("Email Correspondence"), None)])
            .retype_table()
            .unwrap_err();
        assert!(
            format!("{err}").contains("vocabulary does not hold"),
            "{err}"
        );
    }

    #[test]
    fn a_row_that_says_neither_or_both_is_refused() {
        assert!(config_with(&[("A", None, None)]).retype_table().is_err());
        assert!(
            config_with(&[("A", Some("Meeting"), Some("why"))])
                .retype_table()
                .is_err()
        );
    }

    #[test]
    fn a_row_renaming_a_name_to_itself_is_refused() {
        let err = config_with(&[("Meeting", Some("Meeting"), None)])
            .retype_table()
            .unwrap_err();
        assert!(format!("{err}").contains("to itself"), "{err}");
    }

    #[test]
    fn one_old_name_has_one_rule() {
        let err = config_with(&[
            ("Meeting Summary", Some("Meeting"), None),
            ("Meeting Summary", Some("Capture"), None),
        ])
        .retype_table()
        .unwrap_err();
        assert!(format!("{err}").contains("twice"), "{err}");
    }
}
