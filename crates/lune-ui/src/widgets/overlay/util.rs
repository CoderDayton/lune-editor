//! Shared helpers used across overlay sub-modules.

use unicode_segmentation::UnicodeSegmentation;

use crate::primitives::{Block, Borders, Buffer, Rect, Style, Widget};

pub fn pop_grapheme(s: &mut String) {
    let _ = pop_grapheme_returning(s);
}

pub fn pop_grapheme_returning(s: &mut String) -> bool {
    if s.is_empty() {
        return false;
    }
    if let Some((idx, _)) = s.grapheme_indices(true).next_back() {
        s.truncate(idx);
        true
    } else {
        s.pop().is_some()
    }
}

pub fn truncate_inline_text(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let text_len = text.chars().count();
    if text_len <= max_width {
        return text.to_string();
    }

    if max_width == 1 {
        return "…".to_string();
    }

    let mut out: String = text.chars().take(max_width - 1).collect();
    out.push('…');
    out
}

pub fn render_hrule(buf: &mut Buffer, x: u16, y: u16, width: u16) {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::new().dim())
        .render(Rect::new(x, y, width, 1), buf);
}

pub fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().map(|c| c.to_ascii_lowercase());
    let mut bi = b.chars().map(|c| c.to_ascii_lowercase());
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) => match ac.cmp(&bc) {
                std::cmp::Ordering::Equal => {}
                other => return other,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncate_inline_text_adds_ellipsis_for_long_labels() {
        assert_eq!(
            truncate_inline_text("Main stack with wide label", 10),
            "Main stac…"
        );
        assert_eq!(truncate_inline_text("abc", 1), "…");
        assert_eq!(truncate_inline_text("abc", 0), "");
    }
}
