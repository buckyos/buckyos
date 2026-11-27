//! The per-result budget (prompt mechanics TODO B1): how much of the prompt
//! one tool result, or one value a template brings in, may take.

use llm_context::deps::{ByteHeuristicTokenizer, Tokenizer};

/// Tokens one tool result (or one rendered value) takes at most.
pub const RESULT_BUDGET_TOKENS: u32 = 16 * 1024;

/// `text` within `budget` tokens as `tokenizer` counts them: unchanged when
/// it fits, otherwise its head and tail around a marker that says how much
/// was left out and, when `more` is given, how to get the rest
/// (`…[N bytes omitted; <more>]…`). `true` when it was clipped.
pub fn clip_to_budget(
    text: &str,
    budget: u32,
    tokenizer: &dyn Tokenizer,
    more: Option<&str>,
) -> (String, bool) {
    let tokens = tokenizer.count_tokens(text);
    if tokens <= budget {
        return (text.to_string(), false);
    }
    let marker_room = 96 + more.map_or(0, str::len);
    let keep = ((text.len() as u128 * u128::from(budget) / u128::from(tokens.max(1))) as usize)
        .saturating_sub(marker_room);
    let head_end = floor_char_boundary(text, keep / 2);
    let tail_start = ceil_char_boundary(text, text.len() - (keep - keep / 2));
    let tail_start = tail_start.max(head_end);
    let omitted = tail_start - head_end;
    let marker = match more {
        Some(more) => format!("\n…[{omitted} bytes omitted; {more}]…\n"),
        None => format!("\n…[{omitted} bytes omitted]…\n"),
    };
    (
        format!("{}{marker}{}", &text[..head_end], &text[tail_start..]),
        true,
    )
}

/// [`clip_to_budget`] with [`RESULT_BUDGET_TOKENS`] and the runtime's default
/// estimate (4 bytes a token).
pub fn clip_to_default_budget(text: &str, more: Option<&str>) -> (String, bool) {
    clip_to_budget(text, RESULT_BUDGET_TOKENS, &ByteHeuristicTokenizer, more)
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_unchanged_and_clips_head_and_tail() {
        assert_eq!(clip_to_default_budget("short", None), ("short".to_string(), false));
        let big = format!("HEAD{}TAIL", "x".repeat(200 * 1024));
        let (out, clipped) = clip_to_default_budget(&big, Some("full result: /tmp/r"));
        assert!(clipped);
        assert!(out.starts_with("HEAD") && out.ends_with("TAIL"), "{}", &out[..40]);
        assert!(out.contains("bytes omitted; full result: /tmp/r]…"));
        assert!(ByteHeuristicTokenizer.count_tokens(&out) <= RESULT_BUDGET_TOKENS);
    }

    #[test]
    fn keeps_char_boundaries() {
        let big = "汉".repeat(100 * 1024);
        let (out, clipped) = clip_to_default_budget(&big, None);
        assert!(clipped);
        assert!(ByteHeuristicTokenizer.count_tokens(&out) <= RESULT_BUDGET_TOKENS);
    }
}
