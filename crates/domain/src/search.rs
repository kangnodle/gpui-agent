use std::ops::Range;

/// UTF-8 byte ranges using the same literal ASCII case folding as SQLite
/// search.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    text.to_ascii_lowercase()
        .match_indices(&query)
        .map(|(start, _)| start .. start + query.len())
        .collect()
}

pub fn preview(text: &str, query: &str) -> String {
    let query = query.trim().to_ascii_lowercase();
    let Some(found) = text.to_ascii_lowercase().find(&query) else {
        return String::new();
    };
    let after = found + query.len();
    let start = text[.. found].char_indices().rev().nth(39).map_or(0, |(index, _)| index);
    let end = text[after ..].char_indices().nth(80).map_or(text.len(), |(index, _)| after + index);
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        &text[start .. end],
        if end < text.len() { "…" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_preview_and_literal_ranges_keep_valid_byte_boundaries() {
        let text = format!("{}NeEdLe 세계 🦀%_{}", "앞🦀".repeat(100), "뒤🦀".repeat(100));
        let snippet = preview(&text, "needle 세계 🦀%_");
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
        assert!(snippet.chars().count() <= 40 + 13 + 80 + 2);
        let ranges = match_ranges(&snippet, "NEEDLE 세계 🦀%_");
        assert_eq!(ranges.len(), 1);
        assert_eq!(&snippet[ranges[0].clone()], "NeEdLe 세계 🦀%_");
        assert_eq!(
            match_ranges("🦀Hi hi🦀HI", "hi").iter().map(|r| &"🦀Hi hi🦀HI"[r.clone()]).collect::<Vec<_>>(),
            ["Hi", "hi", "HI"]
        );
        assert!(match_ranges("text", " ").is_empty());
        assert_eq!(preview("끝 🦀", "🦀"), "끝 🦀");
    }
}
