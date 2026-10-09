//! Every name in `saver::SAVERS` has a page. The README's saver index is the
//! only way in to the per-saver docs, and a saver added to the table without a
//! row there is undocumented with nothing to say so. lychee (hk) checks the
//! links that exist; this checks that the link exists at all.

use std::path::Path;

const README: &str = include_str!("../README.md");

/// GitHub's heading anchor: lowercased, spaces to `-`, anything else that is
/// not alphanumeric, `-` or `_` dropped.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn every_saver_has_a_doc_page_linked_from_the_readme() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in crate::saver::names() {
        let needle = format!("[`{name}`](");
        let at = README
            .find(&needle)
            .unwrap_or_else(|| panic!("README.md has no [`{name}`](…) link in its saver index"));
        let rest = &README[at + needle.len()..];
        let target = &rest[..rest.find(')').expect("unclosed link")];
        let (file, anchor) = target.split_once('#').unwrap_or((target, ""));
        assert!(
            file.starts_with("docs/savers/"),
            "{name} links to {file}, not a docs/savers/ page"
        );
        let page = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|e| panic!("{name}: {file}: {e}"));
        if !anchor.is_empty() {
            assert!(
                page.lines()
                    .filter_map(|l| l.strip_prefix('#'))
                    .any(|h| slug(h.trim_start_matches('#')) == anchor),
                "{name}: {file} has no heading for #{anchor}"
            );
        }
    }
}

#[test]
fn slug_matches_github() {
    assert_eq!(slug(" `night-coast`"), "night-coast");
    assert_eq!(
        slug(" ascii.rest halftone scenes"),
        "asciirest-halftone-scenes"
    );
    assert_eq!(slug(" At any size"), "at-any-size");
}
