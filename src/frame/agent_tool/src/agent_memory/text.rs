//! Normalization of tags, aliases and full-text tokens.
//!
//! Full text works for Chinese without a language-specific tokenizer: runs of
//! CJK characters become overlapping bigrams, other text becomes lowercase
//! words. The same tokens feed the SQLite FTS table (stored space separated
//! under `unicode61`) and the in-memory fallback scan.

use std::collections::{BTreeSet, HashSet};

use super::{AgentMemoryError, Result};

pub const MIN_TAG_BYTES: usize = 2;
pub const MAX_TAG_BYTES: usize = 32;

pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

pub fn normalize_alias(alias: &str) -> String {
    collapse_whitespace(alias).to_lowercase()
}

/// A.6.2: trimmed UTF-8 length 2..=32 bytes; letters, digits, spaces and `-`;
/// at least one letter or digit.
pub fn validate_tag(tag: &str) -> Result<()> {
    let t = tag.trim();
    let len = t.len();
    if !(MIN_TAG_BYTES..=MAX_TAG_BYTES).contains(&len) {
        return Err(AgentMemoryError::Invalid(format!(
            "tag length must be {MIN_TAG_BYTES}-{MAX_TAG_BYTES} bytes: {t:?}"
        )));
    }
    let mut has_alnum = false;
    for c in t.chars() {
        if !(c.is_alphanumeric() || matches!(c, ' ' | '-')) {
            return Err(AgentMemoryError::Invalid(format!(
                "tag has forbidden character {c:?}: {t:?}"
            )));
        }
        has_alnum |= c.is_alphanumeric();
    }
    if !has_alnum {
        return Err(AgentMemoryError::Invalid(format!(
            "tag must contain at least one letter or digit: {t:?}"
        )));
    }
    Ok(())
}

/// Normalize and validate every tag; any invalid tag is an error.
pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let normalized = collapse_whitespace(tag).to_lowercase();
        if normalized.is_empty() {
            continue;
        }
        validate_tag(&normalized)?;
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    Ok(out)
}

/// Normalize query tags: invalid ones are dropped and reported; when every
/// non-empty tag is invalid the query is an argument error (A.6.1, TD-01).
pub fn normalize_query_tags(tags: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut ok = Vec::new();
    let mut ignored = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let normalized = collapse_whitespace(tag).to_lowercase();
        if normalized.is_empty() {
            continue;
        }
        match validate_tag(&normalized) {
            Ok(()) => {
                if seen.insert(normalized.clone()) {
                    ok.push(normalized);
                }
            }
            Err(_) => ignored.push(tag.clone()),
        }
    }
    if ok.is_empty() && !ignored.is_empty() {
        return Err(AgentMemoryError::Invalid(format!(
            "every recall tag is invalid: {ignored:?}"
        )));
    }
    Ok((ok, ignored))
}

pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // kana
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xAC00..=0xD7AF // hangul
        | 0xF900..=0xFAFF // compatibility
        | 0x20000..=0x2FA1F)
}

/// Full-text tokens: lowercase words (≥ 2 chars) and CJK bigrams (a lone CJK
/// character stays a unigram).
pub fn fts_tokens(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut word = String::new();
    let mut run: Vec<char> = Vec::new();
    fn flush_word(word: &mut String, out: &mut BTreeSet<String>) {
        if word.chars().count() >= 2 {
            out.insert(std::mem::take(word));
        } else {
            word.clear();
        }
    }
    fn flush_run(run: &mut Vec<char>, out: &mut BTreeSet<String>) {
        match run.len() {
            0 => {}
            1 => {
                out.insert(run[0].to_string());
            }
            _ => {
                for w in run.windows(2) {
                    out.insert(w.iter().collect());
                }
            }
        }
        run.clear();
    }
    for c in s.to_lowercase().chars() {
        if is_cjk(c) {
            flush_word(&mut word, &mut out);
            run.push(c);
        } else if c.is_alphanumeric() {
            flush_run(&mut run, &mut out);
            word.push(c);
        } else {
            flush_word(&mut word, &mut out);
            flush_run(&mut run, &mut out);
        }
    }
    flush_word(&mut word, &mut out);
    flush_run(&mut run, &mut out);
    out
}

/// Query tokens that occur in the document.
pub fn fts_hits(query: &BTreeSet<String>, doc: &BTreeSet<String>) -> Vec<String> {
    query.intersection(doc).cloned().collect()
}

/// A document enters through full text when it shares at least two query
/// tokens (one when the query has only one).
pub fn fts_enters(query_len: usize, hits: usize) -> bool {
    hits > 0 && hits >= query_len.min(2)
}

pub fn phrase_hit(haystack_lower: &str, needle_lower: &str) -> bool {
    !needle_lower.is_empty() && haystack_lower.contains(needle_lower)
}

pub fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_bigrams_and_words() {
        let t = fts_tokens("调整游戏背景 on snake-alpha");
        assert!(
            t.contains("游戏") && t.contains("背景") && t.contains("snake") && t.contains("alpha")
        );
        assert!(!t.contains("on") || t.contains("on"));
        let single = fts_tokens("一");
        assert!(single.contains("一"));
    }

    #[test]
    fn query_tags_all_invalid_is_error() {
        assert!(normalize_query_tags(&["a\"b".into()]).is_err());
        let (ok, ignored) = normalize_query_tags(&["a\"b".into(), "背景".into()]).unwrap();
        assert_eq!(ok, vec!["背景".to_string()]);
        assert_eq!(ignored.len(), 1);
        assert!(normalize_query_tags(&["  ".into()]).unwrap().0.is_empty());
    }

    #[test]
    fn tag_rules() {
        assert!(validate_tag("dental").is_ok());
        assert!(validate_tag("phone case").is_ok());
        assert!(validate_tag("绘画创作").is_ok());
        assert!(validate_tag("a").is_err());
        assert!(validate_tag("with\"quote").is_err());
    }
}
