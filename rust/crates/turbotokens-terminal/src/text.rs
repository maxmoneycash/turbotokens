use std::borrow::Cow;

/// Make control characters in an untrusted terminal label visible. Apply this
/// before adding application colors or layout newlines; JSON keeps the original
/// value. Ordinary labels, including printable Unicode, need no allocation.
pub fn escape_terminal_text(value: &str) -> Cow<'_, str> {
    let Some((first_control, _)) = value.char_indices().find(|(_, ch)| ch.is_control()) else {
        return Cow::Borrowed(value);
    };
    let mut escaped = String::with_capacity(value.len());
    escaped.push_str(&value[..first_control]);
    for ch in value[first_control..].chars() {
        if ch.is_control() {
            escaped.extend(ch.escape_default());
        } else {
            escaped.push(ch);
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_labels_borrow_the_original_unicode_text() {
        for label in ["", "claude-sonnet-4", "project/日本語 e\u{301} 🚀"] {
            assert!(matches!(escape_terminal_text(label), Cow::Borrowed(_)));
            assert_eq!(escape_terminal_text(label), label);
        }
    }

    #[test]
    fn controls_are_visible_and_cannot_start_terminal_commands() {
        let label = "m\n\t\r\0\x1b[2J\x1b]0;title\x07\u{7f}\u{85}\u{9b}";
        let escaped = escape_terminal_text(label);
        assert_eq!(
            escaped,
            "m\\n\\t\\r\\u{0}\\u{1b}[2J\\u{1b}]0;title\\u{7}\\u{7f}\\u{85}\\u{9b}"
        );
        assert!(!escaped.chars().any(char::is_control));
    }
}
