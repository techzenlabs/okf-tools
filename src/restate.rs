//! The four restatement gates: what a promoted page may not say.
//!
//! [`crate::promote`] catches pointers — a link into a profile, a citation of
//! a meeting, a target the destination does not hold. Its own header says the
//! other half cannot be checked: "a sentence carrying a read on somebody is
//! forbidden by the same rule that forbids the profile, and no checker sees
//! it."
//!
//! That was true of characterisation and it was over-claimed for four classes
//! beside it. Each of the four is a comparison rather than a judgement, and
//! `okf-promote --propose` has both texts in hand:
//!
//! * **A confidence label the source does not carry.** The pilot promotion
//!   labelled twenty-two claims where the source labelled six, and all sixteen
//!   additions said `Confirmed`. The uniform direction is the tell: a habit
//!   rather than sixteen decisions. One of the sixteen was attached to a claim
//!   the source never makes.
//! * **A person-shaped bullet.** The pilot carried a bullet about a named
//!   vendor engineer that paraphrased his profile's working notes and
//!   published his remote access to a live clinical system. Every name with a
//!   profile in the source is known here, and a bullet is a shape.
//! * **A bare register identifier.** `DEC-066`, `OQ-095`. They resolve only
//!   inside the private registers, so they cite evidence the reader cannot
//!   reach, and the numbers disclose the size of those registers. `OQ-095` in
//!   the pilot pointed at the exact passage the drafter had cut.
//! * **A reconstructed meeting.** "Raised with the vendor at COO level on July
//!   30, 2026 and worked through on a joint call on July 31" names a private
//!   communication, its date and its subject, in a sentence, which is the
//!   disclosure the citation rule exists to suppress. A dated claim is
//!   required; a dated meeting is not.
//!
//! **All four read the body and never the front matter.** `owner` is a
//! sequence of `- name:` records, which is list-shaped and full of people, and
//! `promoted_from` carries a path that would read as an identifier.
//!
//! **All four skip fenced code.** A promoted page quoting a log line or a
//! query is quoting a machine, and a machine's output is not a restatement.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use fancy_regex::Regex;

#[expect(
    clippy::expect_used,
    reason = "static pattern literals, all forced by tests::every_pattern_compiles"
)]
fn compiled(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex literal must compile")
}

/// The confidence labels the promotion pattern defines, and only these.
///
/// Capitalised, because a label is a label and `confirmed` in a sentence is a
/// verb. Counting is deliberately blind to where they appear: a restatement
/// may reword whatever it likes, and the only thing it may not do is end up
/// with more labels than it started with.
pub const LABELS: &[&str] = &[
    "Confirmed",
    "Assumed",
    "Needs confirmation",
    "Needs-confirmation",
];

/// Prefixes whose expansion a reader can reach without the private registers.
///
/// A public standard is not a citation of anything, and `AES-256` has the same
/// shape as `DEC-066`. Extended per-bundle by
/// `[promote] public_identifier_prefixes`.
pub const PUBLIC_PREFIXES: &[&str] = &[
    "AES", "ANSI", "CVE", "CWE", "DICOM", "EN", "FIPS", "HL", "HTTP", "IEC", "IEEE", "ISO", "NIST",
    "OWASP", "RFC", "RSA", "SHA", "TLS", "UTF",
];

static FENCE: LazyLock<Regex> = LazyLock::new(|| compiled(r"(?m)^\s*(?:```|~~~)"));

static BULLET: LazyLock<Regex> = LazyLock::new(|| compiled(r"^\s*(?:[-*+]|\d+[.)])\s+\S"));

static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| compiled(r"\b([A-Z][A-Z0-9]{1,7})-(\d{2,6})\b"));

/// A date a reader could put in a calendar, in the three shapes this estate
/// writes. A bare year is not one: "the 2024 excess" is a claim about a year,
/// not an appointment.
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    compiled(
        r"(?i)\b(?:\d{4}-\d{2}-\d{2}|\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\b|(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2}\b)",
    )
});

/// Words that make a sentence about an occasion rather than about a fact.
///
/// Kept short and kept concrete. "Review" is absent on purpose: an
/// architecture review is a document in this estate, and a gate that fires on
/// the word would fire on half the corpus.
static OCCASION: LazyLock<Regex> = LazyLock::new(|| {
    compiled(
        r"(?i)\b(?:call|calls|meeting|meetings|standup|stand-up|sync|huddle|conversation|conversations|thread|escalation|escalated|discussed|spoke|exchanged|raised with|walked through|worked through)\b",
    )
});

/// One thing a promoted page says that it may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 1-based line in the draft, or 0 for a finding about the whole page.
    pub line: usize,
    /// The label, the name, the identifier — whatever was found.
    pub subject: String,
    /// The line it was found in, whitespace collapsed.
    pub sentence: String,
    /// What the reviewer has to decide, in a sentence.
    pub detail: String,
}

/// The body of a document, with front matter and fenced code blanked out.
///
/// Blanked rather than removed, so every byte offset still maps to the line it
/// came from. A reviewer given the wrong line number reads the wrong sentence
/// and concludes the gate is broken.
#[must_use]
pub fn readable_body(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lines = text.lines();
    let mut in_frontmatter = false;

    // A leading `---` opens front matter. A `---` anywhere else is a thematic
    // break and opens nothing.
    let mut first = true;
    let mut fenced = false;
    let mut pending: Vec<String> = Vec::new();
    for line in lines.by_ref() {
        if first {
            first = false;
            if line.trim_end() == "---" {
                in_frontmatter = true;
                pending.push(String::new());
                continue;
            }
        }
        if in_frontmatter {
            pending.push(String::new());
            if line.trim_end() == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        if FENCE.is_match(line).unwrap_or(false) {
            fenced = !fenced;
            pending.push(String::new());
            continue;
        }
        pending.push(if fenced {
            String::new()
        } else {
            line.to_owned()
        });
    }
    out.append(&mut pending);
    out.join("\n")
}

/// Labels the draft carries and the source does not.
///
/// A count, not a diff. The restatement rewrites the sentence — that is what
/// it is for — so the labels cannot be matched to each other. What can be
/// compared is how many there are, and a promotion that ends with more
/// confidence than it started with has manufactured the difference.
#[must_use]
pub fn manufactured_labels(draft: &str, source: &str) -> Vec<Finding> {
    let drafted = readable_body(draft);
    let sourced = readable_body(source);
    let mut findings = Vec::new();
    for label in LABELS {
        let in_draft = occurrences(&drafted, label);
        let in_source = occurrences(&sourced, label).len();
        if in_draft.len() <= in_source {
            continue;
        }
        let lines: Vec<String> = in_draft.iter().map(|(line, _)| line.to_string()).collect();
        findings.push(Finding {
            line: in_draft.first().map_or(0, |(line, _)| *line),
            subject: (*label).to_owned(),
            sentence: in_draft
                .first()
                .map_or(String::new(), |(_, text)| text.clone()),
            detail: format!(
                "the draft labels {} claim(s) `{label}` and the source labels {in_source}. A \
                 restatement carries the label its source already carries; it does not add one. \
                 Lines: {}.",
                in_draft.len(),
                lines.join(", ")
            ),
        });
    }
    findings
}

/// Every line holding `needle`, 1-based, with the line's text.
fn occurrences(body: &str, needle: &str) -> Vec<(usize, String)> {
    body.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(index, line)| (index + 1, collapse(line)))
        .collect()
}

fn collapse(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bullets naming a person the source keeps a profile on.
///
/// The rule the pilot review settled: a person-shaped bullet does not survive
/// promotion, even when the person is at the vendor and even when the facts in
/// it are system facts. Restate the system fact without the custodian.
///
/// A name in running prose is not caught here and is not meant to be — "the
/// vendor's engineer confirmed it" needs no name and a reviewer reads for
/// that. The bullet is the shape that carries a mini-profile, which is what
/// happened.
#[must_use]
pub fn person_bullets(draft: &str, profiled: &[String]) -> Vec<Finding> {
    let body = readable_body(draft);
    let mut findings = Vec::new();
    for (index, line) in body.lines().enumerate() {
        if !BULLET.is_match(line).unwrap_or(false) {
            continue;
        }
        for name in profiled {
            if name.is_empty() || !line.contains(name.as_str()) {
                continue;
            }
            findings.push(Finding {
                line: index + 1,
                subject: name.clone(),
                sentence: collapse(line),
                detail: format!(
                    "`{name}` has a profile in the source, and this is a bullet about them. A \
                     person-shaped bullet does not survive promotion even when every fact in it \
                     is a system fact: restate the system fact without the custodian. If the \
                     person is genuinely the answer to \"who owns this\", they belong in `owner` \
                     and nowhere else."
                ),
            });
            break;
        }
    }
    findings
}

/// Register identifiers the destination bundle cannot resolve.
///
/// `resolvable` is the destination's own text: a glossary entry, or an earlier
/// promoted page that spells the register out, makes the identifier reachable
/// and the finding goes away. `public` extends [`PUBLIC_PREFIXES`], because
/// `AES-256` has the shape of a citation and cites nothing.
#[must_use]
pub fn unresolvable_identifiers(draft: &str, resolvable: &str, public: &[String]) -> Vec<Finding> {
    let body = readable_body(draft);
    let exempt: BTreeSet<String> = PUBLIC_PREFIXES
        .iter()
        .map(|p| (*p).to_owned())
        .chain(public.iter().map(|p| p.to_uppercase()))
        .collect();
    let mut seen = BTreeSet::new();
    let mut findings = Vec::new();
    for (index, line) in body.lines().enumerate() {
        for capture in IDENTIFIER.captures_iter(line).flatten() {
            let (Some(whole), Some(prefix)) = (capture.get(0), capture.get(1)) else {
                continue;
            };
            if exempt.contains(prefix.as_str()) || resolvable.contains(whole.as_str()) {
                continue;
            }
            if !seen.insert(whole.as_str().to_owned()) {
                continue;
            }
            findings.push(Finding {
                line: index + 1,
                subject: whole.as_str().to_owned(),
                sentence: collapse(line),
                detail: format!(
                    "`{}` resolves only where the reader cannot go, and the number itself says \
                     how large that register is. State what the entry decided, in the sentence, \
                     and drop the identifier — or add the register to this bundle so the \
                     identifier is a link.",
                    whole.as_str()
                ),
            });
        }
    }
    findings
}

/// Sentences that name an occasion and put a date on it.
///
/// The citation rule drops a link to a meeting. A sentence that says the
/// meeting happened, when, and what it was about, puts the disclosure back in
/// prose. The escape is not to hide the date: it is to make the claim about
/// the system rather than about the conversation.
#[must_use]
pub fn reconstructed_meetings(draft: &str) -> Vec<Finding> {
    let body = readable_body(draft);
    let mut findings = Vec::new();
    for (index, line) in body.lines().enumerate() {
        let occasion = OCCASION.find(line).ok().flatten();
        let date = DATE.find(line).ok().flatten();
        let (Some(occasion), Some(date)) = (occasion, date) else {
            continue;
        };
        findings.push(Finding {
            line: index + 1,
            subject: occasion.as_str().to_owned(),
            sentence: collapse(line),
            detail: format!(
                "this dates an occasion — `{}` on `{}` — which reconstructs the meeting the \
                 citation rule dropped. A dated claim is required and a dated meeting is not: \
                 say what is true of the system and when it became true, not who was in the \
                 room. \"At COO level\" does not de-identify an organisation with one COO.",
                occasion.as_str(),
                date.as_str()
            ),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `LazyLock<Regex>` in this file, forced. The patterns are
    /// literals and a bad one is a panic at first use rather than at build,
    /// which in a gate means the gate is missing rather than loud.
    #[test]
    fn every_pattern_compiles() {
        for pattern in [&*FENCE, &*BULLET, &*IDENTIFIER, &*DATE, &*OCCASION] {
            assert!(!pattern.as_str().is_empty());
        }
    }

    const SOURCE: &str =
        "---\ntype: \"System\"\n---\n\nThe relay drops frames (Confirmed, 2026-03-04).\n";

    #[test]
    fn front_matter_and_fenced_code_are_not_the_body() {
        let text = "---\nowner:\n  - name: \"Dana Quill\"\n---\n\nA line.\n\n```\n- Dana Quill ran it\n```\n\nAnother.\n";
        let body = super::readable_body(text);
        assert!(!body.contains("Dana Quill"), "{body}");
        assert!(body.contains("A line."));
        assert!(body.contains("Another."));
        // Line numbers survive blanking, which is the whole reason for it.
        assert_eq!(body.lines().count(), text.lines().count());
        assert_eq!(body.lines().nth(5), Some("A line."));
    }

    /// A thematic break in the middle of a page is not a second front matter
    /// fence. Getting this wrong blanks the rest of the document and every
    /// gate reports clean.
    #[test]
    fn a_thematic_break_does_not_open_front_matter() {
        let body = super::readable_body("# Title\n\n---\n\nStill the body.\n");
        assert!(body.contains("Still the body."));
    }

    #[test]
    fn a_label_the_source_does_not_carry_is_reported() {
        let draft = "---\ntype: \"System\"\n---\n\nThe relay drops frames (Confirmed, 2026-03-04).\n\nIt was corrected the same day (Confirmed, 2026-03-04).\n";
        let found = manufactured_labels(draft, SOURCE);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].subject, "Confirmed");
        assert!(
            found[0].detail.contains("labels 2 claim(s)"),
            "{:?}",
            found[0]
        );
    }

    /// The count is the comparison, so a restatement that rewords everything
    /// and adds nothing passes. This is what makes the gate usable at all.
    #[test]
    fn a_reworded_restatement_carrying_the_same_labels_passes() {
        let draft = "---\ntype: \"System\"\n---\n\nFrames are dropped by the relay under load (Confirmed, 2026-03-04).\n";
        assert!(manufactured_labels(draft, SOURCE).is_empty());
    }

    /// Removing a label is not a finding. Promotion may say less.
    #[test]
    fn dropping_a_label_is_not_a_finding() {
        let draft = "---\ntype: \"System\"\n---\n\nFrames are dropped by the relay under load.\n";
        assert!(manufactured_labels(draft, SOURCE).is_empty());
    }

    #[test]
    fn a_bullet_naming_a_profiled_person_is_reported() {
        let profiled = vec!["Dana Quill".to_owned(), "Ivo Marsh".to_owned()];
        let draft = "---\ntype: \"System\"\n---\n\n- Vendor-side notes: Dana Quill holds the only copy of the backup.\n- The relay drops frames under load.\n";
        let found = person_bullets(draft, &profiled);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].subject, "Dana Quill");
        assert_eq!(found[0].line, 5);
    }

    /// `owner` is a list of people and it is the one place they belong. It is
    /// front matter, so the body scan never sees it.
    #[test]
    fn the_owner_record_is_not_a_person_bullet() {
        let profiled = vec!["Dana Quill".to_owned()];
        let draft = "---\nowner:\n  - name: \"Dana Quill\"\n    title: \"Platform lead\"\n---\n\nThe relay drops frames.\n";
        assert!(person_bullets(draft, &profiled).is_empty());
    }

    #[test]
    fn a_bare_register_identifier_is_reported_and_a_public_one_is_not() {
        let draft = "---\ntype: \"System\"\n---\n\nSee DEC-066 and OQ-095. Traffic is AES-256 and the parser follows RFC-2119.\n";
        let found = unresolvable_identifiers(draft, "", &[]);
        let subjects: Vec<&str> = found.iter().map(|f| f.subject.as_str()).collect();
        assert_eq!(subjects, ["DEC-066", "OQ-095"], "{found:?}");
    }

    /// The destination resolving it is the way out, and it is the right way
    /// out: an identifier the reader can follow is a citation rather than a
    /// gesture at one.
    #[test]
    fn an_identifier_the_destination_holds_is_reachable() {
        let draft = "---\ntype: \"System\"\n---\n\nSee DEC-066.\n";
        let glossary = "DEC-066 — the decision to run one relay per site.";
        assert!(unresolvable_identifiers(draft, glossary, &[]).is_empty());
    }

    #[test]
    fn a_configured_prefix_is_exempt_case_insensitively() {
        let draft = "---\ntype: \"System\"\n---\n\nTracked as JIRA-4102.\n";
        assert!(unresolvable_identifiers(draft, "", &["jira".to_owned()]).is_empty());
    }

    #[test]
    fn a_dated_occasion_is_reported_and_a_dated_fact_is_not() {
        let draft = "---\ntype: \"System\"\n---\n\nWorked through on a joint call on July 31, 2026.\n\nThe relay was cut over on 2026-07-31.\n";
        let found = reconstructed_meetings(draft);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line, 5);
        assert!(found[0].sentence.contains("joint call"));
    }

    /// A year on its own is not an appointment. The pilot page states an
    /// excess for 2023 and for 2024 in sentences that also say "discussed",
    /// and blocking those would make the gate the thing people route around.
    #[test]
    fn a_bare_year_is_not_a_date() {
        let draft = "---\ntype: \"System\"\n---\n\nThe 2024 excess was discussed at length in the register.\n";
        assert!(reconstructed_meetings(draft).is_empty());
    }

    /// A quoted log line is a machine's output, not a restatement.
    #[test]
    fn fenced_code_is_not_prose() {
        let draft =
            "---\ntype: \"System\"\n---\n\n```\n2026-07-31 joint call scheduled\nDEC-066\n```\n";
        assert!(reconstructed_meetings(draft).is_empty());
        assert!(unresolvable_identifiers(draft, "", &[]).is_empty());
    }
}
