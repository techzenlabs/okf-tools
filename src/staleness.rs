//! `stale_after`, and the committed day it is measured against.
//!
//! The field is OKF v0.2's staleness slot, and until now this tool only
//! checked its *shape*. A document could say `stale_after: 2020-01-01` and
//! nothing anywhere would ever say so, which is worse than not having the
//! field: an author writing it reasonably believes the document will be
//! flagged when it goes stale.
//!
//! ## Why there is no clock in this file
//!
//! The obvious implementation compares `stale_after` to today. It is also the
//! defect this module exists to avoid. `okf-check` runs inside a nix
//! derivation cached on its inputs, so a verdict that reads the wall clock is
//! computed once and then served from cache: nothing in the repository
//! changes, the answer changes anyway, and a green default branch stops being
//! evidence that the check passes today. That class was swept across this
//! estate and found twenty-one times, four of which had already bitten
//! somebody. This one is not going to be the twenty-second.
//!
//! So the day is **data, not ambient state**. A bundle commits a one-line
//! [`AS_OF_FILE`] naming the day its gates resolve to, that file is an input
//! to the derivation like every other tracked file, and the comparison is a
//! pure function of two committed strings. The cached green then says
//! something true and checkable — "as of the day this bundle committed to,
//! nothing had lapsed" — rather than something about a build machine's
//! calendar.
//!
//! What makes the real answer move is the day moving: bumping `.gate-as-of`
//! is a one-line diff whose gate is red if something lapsed, addressed to
//! whoever owns the bump rather than ambushing whoever next touched the
//! source tree. `okf-check --as-of=<day>` answers the same question without
//! committing anything, which is what a scheduled bump job or a person runs.

use std::fmt;
use std::path::Path;

/// The file naming the day this bundle's staleness comparisons resolve to.
///
/// Repository root, beside `okf.toml`, and deliberately the same name the
/// estate's other date-reading gates take, so a repository has one day to
/// bump rather than one per tool.
pub const AS_OF_FILE: &str = ".gate-as-of";

/// One calendar day, `YYYY-MM-DD`.
///
/// A validated newtype rather than a `String`, so nothing downstream can
/// compare a date against `soon`. Days in this shape sort lexicographically,
/// which is what lets the comparison be a string compare with no date library
/// and no clock behind it.
///
/// Month and day are range-checked but the calendar is not: `2026-02-30` is
/// accepted. Ordering is what this type is for, and a day that cannot exist
/// still orders correctly against the days that can.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day(String);

impl Day {
    /// Parse a day, or `None` if `text` is not one.
    ///
    /// Surrounding whitespace is trimmed, because the file this reads is
    /// written by `date -u +%F > .gate-as-of` as often as by hand.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (year, rest) = text.split_once('-')?;
        let (month, day) = rest.split_once('-')?;
        digits(year, 4)?;
        let month = digits(month, 2)?;
        let day = digits(day, 2)?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Self(text.to_owned()))
    }

    /// Has this day passed, measured as of `as_of`?
    ///
    /// `stale_after` names the last day the document is good for, so the day
    /// itself is not yet stale. Strictly later, and the boundary has a
    /// fixture.
    #[must_use]
    pub fn has_passed(&self, as_of: &Self) -> bool {
        as_of > self
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Exactly `len` ASCII digits, as a number.
fn digits(text: &str, len: usize) -> Option<u32> {
    if text.len() != len || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[derive(Debug, thiserror::Error)]
pub enum AsOfError {
    #[error("{path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{path}: `{found}` is not a YYYY-MM-DD day. This file names the day \
         every staleness comparison in this bundle resolves to, and a gate \
         that cannot read its own date must not pass."
    )]
    Malformed { path: String, found: String },
}

/// Read the day `repo_root` has committed to, if it has committed to one.
///
/// `Ok(None)` means the file is absent, which is the state of every bundle
/// that has not adopted the convention. That is not an error: a bundle with
/// no `stale_after` anywhere in it has nothing to measure, and forcing a date
/// file on it would be a gate about a field it does not use. A bundle that
/// *does* carry the field and has no day gets told so per document, by
/// [`crate::check`], where the author can see it.
///
/// A file that exists and is not a day is an error, and a hard one. Failing
/// open there would turn a typo into silence, which is the whole defect.
///
/// # Errors
///
/// Fails when the file exists and cannot be read, or holds something that is
/// not a `YYYY-MM-DD` day.
pub fn read(repo_root: &Path) -> Result<Option<Day>, AsOfError> {
    let path = repo_root.join(AS_OF_FILE);
    let shown = path.display().to_string();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AsOfError::Read {
                path: shown,
                source,
            });
        }
    };
    Day::parse(&text).map(Some).ok_or(AsOfError::Malformed {
        found: text.trim().to_owned(),
        path: shown,
    })
}

#[cfg(test)]
mod tests {
    use super::{AS_OF_FILE, Day, read};

    #[test]
    fn a_day_parses_only_in_the_one_shape() {
        assert!(Day::parse("2026-06-15").is_some());
        assert!(Day::parse(" 2026-06-15\n").is_some());
        for bad in [
            "soon",
            "2026-6-15",
            "26-06-15",
            "2026-06-15T00:00:00Z",
            "2026-13-01",
            "2026-00-01",
            "2026-06-32",
            "2026-06-00",
            "2026-06-15-01",
            "",
        ] {
            assert!(Day::parse(bad).is_none(), "{bad} parsed");
        }
    }

    /// The boundary, stated twice: the named day is the last good one.
    #[test]
    fn stale_after_names_the_last_good_day() {
        let stale = Day::parse("2026-06-15").unwrap_or_else(|| unreachable!());
        let same = Day::parse("2026-06-15").unwrap_or_else(|| unreachable!());
        let next = Day::parse("2026-06-16").unwrap_or_else(|| unreachable!());
        let before = Day::parse("2026-06-14").unwrap_or_else(|| unreachable!());
        assert!(!stale.has_passed(&before));
        assert!(!stale.has_passed(&same));
        assert!(stale.has_passed(&next));
    }

    /// Lexicographic order is calendar order across a year boundary, which is
    /// the property the string compare rests on.
    #[test]
    fn days_sort_as_the_calendar_does() {
        let mut days: Vec<Day> = ["2026-12-31", "2026-01-02", "2027-01-01", "2026-01-10"]
            .iter()
            .filter_map(|d| Day::parse(d))
            .collect();
        days.sort();
        let order: Vec<&str> = days.iter().map(Day::as_str).collect();
        assert_eq!(
            order,
            ["2026-01-02", "2026-01-10", "2026-12-31", "2027-01-01"]
        );
    }

    #[test]
    fn an_absent_file_is_not_an_error_and_a_malformed_one_is() {
        let dir = std::env::temp_dir().join(format!("okf-asof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let Ok(()) = std::fs::create_dir_all(&dir) else {
            unreachable!("temp dir")
        };
        assert!(matches!(read(&dir), Ok(None)));

        let _ = std::fs::write(dir.join(AS_OF_FILE), "2026-06-15\n");
        assert_eq!(
            read(&dir).ok().flatten().map(|d| d.to_string()),
            Some("2026-06-15".to_owned())
        );

        let _ = std::fs::write(dir.join(AS_OF_FILE), "whenever\n");
        let message = read(&dir).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            message.contains("`whenever` is not a YYYY-MM-DD day"),
            "{message}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
