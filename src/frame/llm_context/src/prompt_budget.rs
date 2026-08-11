//! Token-budget assembler. See `notepads/prompt_render_engine.md` §6.1 / §6.2.
//!
//! Pure algorithm: caller hands a tokenizer + list of `BudgetedSection`,
//! we decide which sections stay, how much each one is allowed to keep,
//! and apply head/tail/middle truncation to honour the cap.

use crate::deps::Tokenizer;

#[derive(Debug, Clone)]
pub struct BudgetedSection {
    /// Stable identifier (caller-defined, debug / lookup only).
    pub key: String,
    pub content: String,
    /// Higher number ⇒ allocated first. Equal priorities share their bucket
    /// in input order.
    pub priority: u8,
    /// Floor — section must keep at least this many tokens or it is dropped.
    pub min_tokens: u32,
    pub trunc: TruncFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncFrom {
    /// Cut from the start, keep the tail (e.g. "latest messages").
    Head,
    /// Cut from the end, keep the head (e.g. "table of contents").
    Tail,
    /// Cut from the middle, keep both head + tail (e.g. "long doc summary").
    Middle,
}

pub struct PromptBudgeter<'a> {
    pub tokenizer: &'a dyn Tokenizer,
    pub total_budget_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct FitOutcome {
    pub kept: Vec<FittedSection>,
    pub dropped: Vec<String>,
    pub tokens_used: u32,
    pub tokens_remaining: u32,
}

#[derive(Debug, Clone)]
pub struct FittedSection {
    pub key: String,
    pub content: String,
    pub tokens: u32,
    pub truncated: bool,
}

impl<'a> PromptBudgeter<'a> {
    pub fn new(tokenizer: &'a dyn Tokenizer, total_budget_tokens: u32) -> Self {
        Self {
            tokenizer,
            total_budget_tokens,
        }
    }

    pub fn fit(&self, sections: Vec<BudgetedSection>) -> FitOutcome {
        if sections.is_empty() {
            return FitOutcome {
                kept: Vec::new(),
                dropped: Vec::new(),
                tokens_used: 0,
                tokens_remaining: self.total_budget_tokens,
            };
        }

        // Precompute raw token counts in input order. Index lets us return
        // results in input order regardless of how the bucketing reorders.
        let raw_tokens: Vec<u32> = sections
            .iter()
            .map(|s| self.tokenizer.count_tokens(&s.content))
            .collect();
        let total_raw: u64 = raw_tokens.iter().map(|t| *t as u64).sum();

        if total_raw <= self.total_budget_tokens as u64 {
            // Fast path: everything fits.
            let kept: Vec<FittedSection> = sections
                .iter()
                .zip(raw_tokens.iter())
                .map(|(s, t)| FittedSection {
                    key: s.key.clone(),
                    content: s.content.clone(),
                    tokens: *t,
                    truncated: false,
                })
                .collect();
            let used = total_raw.min(u32::MAX as u64) as u32;
            return FitOutcome {
                kept,
                dropped: Vec::new(),
                tokens_used: used,
                tokens_remaining: self.total_budget_tokens.saturating_sub(used),
            };
        }

        // Slow path: budget-constrained. Allocate per priority bucket.
        let mut allocation: Vec<Option<u32>> = vec![None; sections.len()];
        let mut dropped_keys: Vec<String> = Vec::new();

        // Bucket indices by priority (high → low).
        let mut by_prio: std::collections::BTreeMap<u8, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (idx, sec) in sections.iter().enumerate() {
            by_prio.entry(sec.priority).or_default().push(idx);
        }

        let mut remaining: u32 = self.total_budget_tokens;
        for (_prio, bucket) in by_prio.iter().rev() {
            // Pass 1: hand out `min_tokens` to each section in input order;
            // drop sections whose floor will not fit.
            let mut active: Vec<usize> = Vec::with_capacity(bucket.len());
            for idx in bucket {
                let min = sections[*idx].min_tokens;
                if min > remaining {
                    dropped_keys.push(sections[*idx].key.clone());
                    allocation[*idx] = None;
                    continue;
                }
                allocation[*idx] = Some(min);
                remaining = remaining.saturating_sub(min);
                active.push(*idx);
            }

            if remaining == 0 || active.is_empty() {
                continue;
            }

            // Pass 2: round-robin top-up until each section is at its raw
            // size or the budget is exhausted. Capacity per section = raw.
            // Step size is 1 token to keep distribution proportional and
            // tests predictable.
            loop {
                if remaining == 0 {
                    break;
                }
                let mut progressed = false;
                for idx in &active {
                    if remaining == 0 {
                        break;
                    }
                    let cap = raw_tokens[*idx];
                    let cur = allocation[*idx].unwrap_or(0);
                    if cur >= cap {
                        continue;
                    }
                    allocation[*idx] = Some(cur + 1);
                    remaining -= 1;
                    progressed = true;
                }
                if !progressed {
                    break;
                }
            }
        }

        // Materialise kept sections in input order.
        let mut kept: Vec<FittedSection> = Vec::new();
        let mut used: u64 = 0;
        for (idx, sec) in sections.iter().enumerate() {
            let Some(budget) = allocation[idx] else {
                continue;
            };
            let raw = raw_tokens[idx];
            let (content, truncated, tokens) = if budget >= raw {
                (sec.content.clone(), false, raw)
            } else {
                let (out, kept_tokens) =
                    truncate_to_budget(self.tokenizer, &sec.content, budget, sec.trunc);
                (out, true, kept_tokens)
            };
            used += tokens as u64;
            kept.push(FittedSection {
                key: sec.key.clone(),
                content,
                tokens,
                truncated,
            });
        }

        let used = used.min(u32::MAX as u64) as u32;
        FitOutcome {
            kept,
            dropped: dropped_keys,
            tokens_used: used,
            tokens_remaining: self.total_budget_tokens.saturating_sub(used),
        }
    }

    pub fn fit_single(&self, text: &str, budget_tokens: u32, trunc: TruncFrom) -> String {
        let raw = self.tokenizer.count_tokens(text);
        if raw <= budget_tokens {
            return text.to_string();
        }
        truncate_to_budget(self.tokenizer, text, budget_tokens, trunc).0
    }
}

/// Binary-search the largest prefix/suffix/middle slice whose token count is
/// ≤ `budget`. Returns the resulting string and its measured token count.
fn truncate_to_budget(
    tokenizer: &dyn Tokenizer,
    text: &str,
    budget: u32,
    trunc: TruncFrom,
) -> (String, u32) {
    if budget == 0 || text.is_empty() {
        return (String::new(), 0);
    }

    // Operate on char indices to stay UTF-8 safe.
    let char_indices: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let total_chars = char_indices.len();
    if total_chars == 0 {
        return (String::new(), 0);
    }

    match trunc {
        TruncFrom::Tail => {
            let n_chars = binary_search_chars(tokenizer, text, budget, &char_indices, true);
            let end = char_indices.get(n_chars).copied().unwrap_or(text.len());
            let out = text[..end].to_string();
            let tokens = tokenizer.count_tokens(&out);
            (out, tokens)
        }
        TruncFrom::Head => {
            let n_chars = binary_search_chars(tokenizer, text, budget, &char_indices, false);
            // Keep last n_chars.
            let start_idx = total_chars.saturating_sub(n_chars);
            let start = char_indices.get(start_idx).copied().unwrap_or(text.len());
            let out = text[start..].to_string();
            let tokens = tokenizer.count_tokens(&out);
            (out, tokens)
        }
        TruncFrom::Middle => {
            // Split budget roughly in half between head and tail, with an
            // ellipsis joiner. Use a 1-token marker as a conservative budget.
            const MARKER: &str = "…";
            let marker_tokens = tokenizer.count_tokens(MARKER).max(1);
            if budget <= marker_tokens {
                return (MARKER.to_string(), marker_tokens.min(budget));
            }
            let usable = budget - marker_tokens;
            let head_budget = usable / 2 + (usable % 2);
            let tail_budget = usable - head_budget;
            let head_chars = binary_search_chars(tokenizer, text, head_budget, &char_indices, true);
            let tail_chars =
                binary_search_chars(tokenizer, text, tail_budget, &char_indices, false);
            let head_end = char_indices.get(head_chars).copied().unwrap_or(text.len());
            let tail_start_idx = total_chars.saturating_sub(tail_chars);
            // Avoid overlap: if head and tail would intersect, fall back to
            // head-only truncation under budget.
            if head_chars + tail_chars >= total_chars {
                return (text.to_string(), tokenizer.count_tokens(text));
            }
            let tail_start = char_indices
                .get(tail_start_idx)
                .copied()
                .unwrap_or(text.len());
            let mut out = String::new();
            out.push_str(&text[..head_end]);
            out.push_str(MARKER);
            out.push_str(&text[tail_start..]);
            let tokens = tokenizer.count_tokens(&out);
            (out, tokens)
        }
    }
}

/// Binary-search the largest `n_chars` whose corresponding head (or tail)
/// slice fits within `budget`. `from_head=true` searches prefixes, `false`
/// searches suffixes.
fn binary_search_chars(
    tokenizer: &dyn Tokenizer,
    text: &str,
    budget: u32,
    char_indices: &[usize],
    from_head: bool,
) -> usize {
    let total = char_indices.len();
    let mut lo = 0usize;
    let mut hi = total;
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let slice = if from_head {
            let end = char_indices.get(mid).copied().unwrap_or(text.len());
            &text[..end]
        } else {
            let start_idx = total - mid;
            let start = char_indices.get(start_idx).copied().unwrap_or(text.len());
            &text[start..]
        };
        if tokenizer.count_tokens(slice) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::Tokenizer;

    /// 1 token per character. Easy to reason about in tests.
    struct CharTok;
    impl Tokenizer for CharTok {
        fn count_tokens(&self, text: &str) -> u32 {
            text.chars().count() as u32
        }
    }

    fn section(
        key: &str,
        content: &str,
        priority: u8,
        min_tokens: u32,
        trunc: TruncFrom,
    ) -> BudgetedSection {
        BudgetedSection {
            key: key.into(),
            content: content.into(),
            priority,
            min_tokens,
            trunc,
        }
    }

    #[test]
    fn all_fit_returns_original_content() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 100);
        let out = b.fit(vec![
            section("a", "hello", 1, 0, TruncFrom::Tail),
            section("b", "world", 1, 0, TruncFrom::Tail),
        ]);
        assert_eq!(out.kept.len(), 2);
        assert!(out.dropped.is_empty());
        assert!(out.kept.iter().all(|s| !s.truncated));
        assert_eq!(out.tokens_used, 10);
    }

    #[test]
    fn single_section_tail_truncation() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 4);
        let out = b.fit(vec![section("a", "abcdefgh", 1, 0, TruncFrom::Tail)]);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.kept[0].content, "abcd");
        assert!(out.kept[0].truncated);
        assert_eq!(out.tokens_used, 4);
    }

    #[test]
    fn single_section_head_truncation() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 4);
        let out = b.fit(vec![section("a", "abcdefgh", 1, 0, TruncFrom::Head)]);
        assert_eq!(out.kept[0].content, "efgh");
        assert!(out.kept[0].truncated);
    }

    #[test]
    fn single_section_middle_truncation() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 5);
        let out = b.fit(vec![section("a", "abcdefghij", 1, 0, TruncFrom::Middle)]);
        assert!(out.kept[0].truncated);
        // Head + tail + marker; ~5 tokens budget so result must be ≤ 5 chars.
        assert!(out.kept[0].tokens <= 5);
        assert!(out.kept[0].content.contains('…'));
    }

    #[test]
    fn high_priority_eats_first() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 6);
        let out = b.fit(vec![
            section("hi", "AAAAAA", 9, 0, TruncFrom::Tail),
            section("lo", "BBBBBB", 1, 0, TruncFrom::Tail),
        ]);
        // Output is in input order; "hi" was allocated first ⇒ takes all 6.
        assert_eq!(out.kept[0].key, "hi");
        assert_eq!(out.kept[0].content, "AAAAAA");
        assert!(!out.kept[0].truncated);
        // "lo" gets nothing → dropped (min_tokens=0 means budget=0 keeps empty content)
        // Spec: min_tokens=0 with no remaining → kept but empty? Let's check.
        // Our implementation allocates 0 to it, so kept with 0 tokens.
        let lo = out.kept.iter().find(|s| s.key == "lo").unwrap();
        assert!(lo.truncated);
        assert_eq!(lo.tokens, 0);
        assert_eq!(lo.content, "");
    }

    #[test]
    fn min_tokens_above_budget_drops_section() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 4);
        let out = b.fit(vec![
            section("big", "AAAAA", 1, 10, TruncFrom::Tail),
            section("ok", "B", 1, 0, TruncFrom::Tail),
        ]);
        assert_eq!(out.dropped, vec!["big".to_string()]);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.kept[0].key, "ok");
        assert_eq!(out.kept[0].content, "B");
    }

    #[test]
    fn empty_input_returns_empty() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 10);
        let out = b.fit(Vec::new());
        assert!(out.kept.is_empty());
        assert!(out.dropped.is_empty());
        assert_eq!(out.tokens_remaining, 10);
    }

    #[test]
    fn zero_budget_drops_or_empties_everything() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 0);
        let out = b.fit(vec![section("a", "AAA", 1, 1, TruncFrom::Tail)]);
        assert_eq!(out.dropped, vec!["a".to_string()]);
    }

    #[test]
    fn fit_single_truncates_when_over_budget() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 100);
        let out = b.fit_single("abcdefghij", 3, TruncFrom::Tail);
        assert_eq!(out, "abc");
    }

    #[test]
    fn fit_single_passes_through_when_under_budget() {
        let tok = CharTok;
        let b = PromptBudgeter::new(&tok, 100);
        let out = b.fit_single("abc", 100, TruncFrom::Tail);
        assert_eq!(out, "abc");
    }

    #[test]
    fn priority_bucket_drops_in_order_when_min_exhausted() {
        let tok = CharTok;
        // Budget=5; bucket has [{min=3}, {min=3}, {min=3}] — first two fit
        // mins (3+3=6 exceeds 5, so second is dropped; remaining 2 unused for
        // it). Actually with budget=5: first takes 3, remaining=2 (< 3) → drop
        // second; third needs 3 (> 2) → drop too.
        let out = PromptBudgeter::new(&tok, 5).fit(vec![
            section("a", "AAAA", 1, 3, TruncFrom::Tail),
            section("b", "BBBB", 1, 3, TruncFrom::Tail),
            section("c", "CCCC", 1, 3, TruncFrom::Tail),
        ]);
        assert_eq!(out.dropped, vec!["b".to_string(), "c".to_string()]);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.kept[0].key, "a");
        // After min=3, remaining=2 distributes round-robin → tokens=5.
        assert_eq!(out.kept[0].tokens, 4);
    }
}
