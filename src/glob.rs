//! Path globbing for `[[type_rules]]`.
//!
//! A type rule maps a path shape to a `type`, so the matcher has to understand
//! paths rather than filenames: `*` must not cross a `/`, or `plans/*.md`
//! would swallow `plans/active/0606-thing.md` and type an execution plan as a
//! work item.
//!
//! Supported: `?`, `*` within one segment, `**` across segments, and `[...]`
//! classes with ranges and `!` negation. Nothing else, because a rule nobody
//! can read is a rule that mistypes documents quietly.

/// Does the bundle-relative `path` match `pattern`?
#[must_use]
pub fn matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    match_segments(&pattern, &path)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        // Pattern exhausted: a match only if the path is exhausted too.
        None => path.is_empty(),
        Some((&"**", rest)) => {
            // `**` matches zero or more segments, so try every split point.
            // Zero first, which makes `**/x` match a bare `x`.
            (0..=path.len()).any(|take| match_segments(rest, &path[take..]))
        }
        Some((&head, rest)) => match path.split_first() {
            None => false,
            Some((&first, tail)) => match_segment(head, first) && match_segments(rest, tail),
        },
    }
}

/// Match one path segment, where `*` is any run of characters within it.
fn match_segment(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    match_here(&pattern, &name)
}

fn match_here(pattern: &[char], name: &[char]) -> bool {
    let Some((&head, rest)) = pattern.split_first() else {
        return name.is_empty();
    };
    match head {
        '*' => (0..=name.len()).any(|skip| match_here(rest, &name[skip..])),
        '?' => !name.is_empty() && match_here(rest, &name[1..]),
        '[' => match_class(rest, name),
        literal => match name.split_first() {
            Some((&first, tail)) if first == literal => match_here(rest, tail),
            _ => false,
        },
    }
}

/// A `[...]` class, entered just after the opening bracket.
fn match_class(pattern: &[char], name: &[char]) -> bool {
    let Some((&candidate, name_rest)) = name.split_first() else {
        return false;
    };
    let (negated, pattern) = match pattern.split_first() {
        Some((&'!', rest)) => (true, rest),
        _ => (false, pattern),
    };

    let mut index = 0;
    let mut hit = false;
    while index < pattern.len() {
        let Some(&current) = pattern.get(index) else {
            break;
        };
        if current == ']' && index > 0 {
            // Class closed: decide, then carry on with what follows it.
            let tail = pattern.get(index.saturating_add(1)..).unwrap_or_default();
            return (hit != negated) && match_here(tail, name_rest);
        }
        // A range, `a-z`, but only when the `-` is not the class's last char.
        let is_range = pattern.get(index.saturating_add(1)) == Some(&'-')
            && pattern
                .get(index.saturating_add(2))
                .is_some_and(|&c| c != ']');
        if is_range {
            if let Some(&high) = pattern.get(index.saturating_add(2))
                && (current..=high).contains(&candidate)
            {
                hit = true;
            }
            index = index.saturating_add(3);
        } else {
            if current == candidate {
                hit = true;
            }
            index = index.saturating_add(1);
        }
    }
    // Unclosed class: not a match, rather than a panic or a silent pass.
    false
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn a_star_does_not_cross_a_slash() {
        assert!(matches("plans/*.md", "plans/0001-thing.md"));
        assert!(!matches("plans/*.md", "plans/active/0606-thing.md"));
        assert!(matches("plans/active/*.md", "plans/active/0606-thing.md"));
    }

    #[test]
    fn double_star_crosses_segments_including_none() {
        assert!(matches("docs/**/*.md", "docs/a/b/c.md"));
        assert!(matches("docs/**/*.md", "docs/c.md"));
        assert!(matches("**/evidence.md", "a/b/evidence.md"));
        assert!(matches("**/evidence.md", "evidence.md"));
        assert!(matches("generated/**", "generated/a/b.md"));
    }

    #[test]
    fn classes_cover_ranges_and_negation() {
        assert!(matches("plans/[0-9]*.md", "plans/0001-thing.md"));
        assert!(!matches("plans/[0-9]*.md", "plans/README.md"));
        assert!(matches("[!0-9]*.md", "README.md"));
        assert!(!matches("[!0-9]*.md", "0001.md"));
    }

    #[test]
    fn a_literal_pattern_matches_only_itself() {
        assert!(matches("index.md", "index.md"));
        assert!(!matches("index.md", "docs/index.md"));
        assert!(matches("?.md", "a.md"));
        assert!(!matches("?.md", "ab.md"));
    }

    /// An unclosed class must not match and must not panic.
    #[test]
    fn a_malformed_class_is_refused_quietly() {
        assert!(!matches("[0-9.md", "1.md"));
    }
}
