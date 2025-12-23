use chrono::{DateTime, Utc};
use tera::Tera;

use crate::config::{
    DEFAULT_PREVIEW_LINES, DEFAULT_TRUNCATE_WORDS, PREVIEW_HARD_LIMIT,
    SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE, SECONDS_PER_MONTH, SECONDS_PER_YEAR,
    TEMPLATE_GLOB,
};
use crate::error::AppError;

/// Initialize the Tera template engine
pub fn init_templates() -> Result<Tera, AppError> {
    let mut tera = Tera::new(TEMPLATE_GLOB)?;

    // Add custom filters
    tera.register_filter("truncate_words", truncate_words_filter);
    tera.register_filter("timeago", timeago_filter);
    tera.register_filter("preview", preview_filter);
    tera.register_filter("has_more_lines", has_more_lines_filter);

    Ok(tera)
}

/// Truncate text to a certain number of words
fn truncate_words_filter(
    value: &tera::Value,
    args: &std::collections::HashMap<String, tera::Value>,
) -> tera::Result<tera::Value> {
    let s = value
        .as_str()
        .ok_or_else(|| tera::Error::msg("truncate_words filter expects a string"))?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TRUNCATE_WORDS as u64) as usize;

    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() <= count {
        Ok(tera::Value::String(s.to_string()))
    } else {
        let truncated = words[..count].join(" ");
        Ok(tera::Value::String(format!("{}...", truncated)))
    }
}

/// Convert a date string to a human-readable relative time (e.g., "2 hours ago")
fn timeago_filter(
    value: &tera::Value,
    _args: &std::collections::HashMap<String, tera::Value>,
) -> tera::Result<tera::Value> {
    let date_str = value
        .as_str()
        .ok_or_else(|| tera::Error::msg("timeago filter expects a string"))?;

    // Try to parse the date string (RFC 2822 format from NNTP)
    let parsed = DateTime::parse_from_rfc2822(date_str)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| DateTime::parse_from_rfc3339(date_str).map(|dt| dt.with_timezone(&Utc)));

    match parsed {
        Ok(date) => {
            let now = Utc::now();
            let duration = now.signed_duration_since(date);

            let seconds = duration.num_seconds();
            let result = if seconds < 0 {
                "in the future".to_string()
            } else if seconds < SECONDS_PER_MINUTE {
                "just now".to_string()
            } else if seconds < SECONDS_PER_HOUR {
                let mins = seconds / SECONDS_PER_MINUTE;
                if mins == 1 {
                    "1 minute ago".to_string()
                } else {
                    format!("{} minutes ago", mins)
                }
            } else if seconds < SECONDS_PER_DAY {
                let hours = seconds / SECONDS_PER_HOUR;
                if hours == 1 {
                    "1 hour ago".to_string()
                } else {
                    format!("{} hours ago", hours)
                }
            } else if seconds < SECONDS_PER_MONTH {
                let days = seconds / SECONDS_PER_DAY;
                if days == 1 {
                    "1 day ago".to_string()
                } else {
                    format!("{} days ago", days)
                }
            } else if seconds < SECONDS_PER_YEAR {
                let months = seconds / SECONDS_PER_MONTH;
                if months == 1 {
                    "1 month ago".to_string()
                } else {
                    format!("{} months ago", months)
                }
            } else {
                let years = seconds / SECONDS_PER_YEAR;
                if years == 1 {
                    "1 year ago".to_string()
                } else {
                    format!("{} years ago", years)
                }
            };

            Ok(tera::Value::String(result))
        }
        Err(_) => {
            // If parsing fails, return the original string
            Ok(tera::Value::String(date_str.to_string()))
        }
    }
}

/// Check if a line is a quote line (starts with >) or a quote attribution line
/// (e.g., "On Thu, 30 Oct 2025, John Smith wrote:")
fn is_quote_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    
    // Lines starting with > are block quotes
    if trimmed.starts_with('>') {
        return true;
    }
    
    // Check for quote attribution lines like "On <date>, <name> wrote:"
    if trimmed.starts_with("On ") && trimmed.ends_with(':') {
        // Look for common patterns: "wrote:", "writes:", "said:", "says:"
        let lower = trimmed.to_lowercase();
        if lower.ends_with(" wrote:")
            || lower.ends_with(" writes:")
            || lower.ends_with(" said:")
            || lower.ends_with(" says:")
        {
            return true;
        }
    }
    
    // Check for "name <email> writes:" pattern
    if trimmed.ends_with(':') && trimmed.contains('<') && trimmed.contains('>') {
        let lower = trimmed.to_lowercase();
        if lower.ends_with(" wrote:")
            || lower.ends_with(" writes:")
            || lower.ends_with(" said:")
            || lower.ends_with(" says:")
        {
            return true;
        }
    }
    
    false
}

/// Strip block quotes (lines starting with >) from beginning and end of text.
/// Also strips quote attribution lines and adjacent empty lines.
fn strip_block_quotes(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();

    // Find first non-quote line, skipping empty lines adjacent to quotes
    let mut start = 0;
    while start < lines.len() {
        let line = lines[start];
        if is_quote_line(line) || line.trim().is_empty() {
            // Skip quote lines and empty lines at the start
            // But for empty lines, only skip if they're followed by quote lines OR
            // we haven't seen any content yet (still at start)
            if line.trim().is_empty() {
                // Check if this empty line is followed by quote lines
                let next_non_empty = lines[start + 1..].iter().position(|l| !l.trim().is_empty());
                match next_non_empty {
                    Some(offset) if is_quote_line(lines[start + 1 + offset]) => start += 1,
                    None => start += 1, // All remaining lines are empty
                    _ => break, // Next non-empty line is content, stop here but we'll trim below
                }
            } else {
                start += 1;
            }
        } else {
            break;
        }
    }
    
    // Skip any remaining empty lines at the start
    while start < lines.len() && lines[start].trim().is_empty() {
        start += 1;
    }

    // Find last non-quote line, skipping empty lines adjacent to quotes
    let mut end = lines.len();
    while end > start {
        let line = lines[end - 1];
        if is_quote_line(line) || line.trim().is_empty() {
            if line.trim().is_empty() {
                // Check if this empty line is preceded by quote lines
                let prev_non_empty = lines[..end - 1].iter().rposition(|l| !l.trim().is_empty());
                match prev_non_empty {
                    Some(idx) if is_quote_line(lines[idx]) => end -= 1,
                    None => end -= 1, // All preceding lines are empty
                    _ => break, // Previous non-empty line is content, stop here but we'll trim below
                }
            } else {
                end -= 1;
            }
        } else {
            break;
        }
    }
    
    // Skip any remaining empty lines at the end
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }

    if start >= end {
        return String::new();
    }

    lines[start..end].join("\n")
}

/// Truncate text to a preview of N lines, stopping at next line break if over,
/// with a hard limit of 1024 characters. Block quotes are stripped first.
fn preview_filter(
    value: &tera::Value,
    args: &std::collections::HashMap<String, tera::Value>,
) -> tera::Result<tera::Value> {
    let s = value
        .as_str()
        .ok_or_else(|| tera::Error::msg("preview filter expects a string"))?;

    let max_lines = args
        .get("lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_PREVIEW_LINES as u64) as usize;

    // Strip block quotes first
    let stripped = strip_block_quotes(s);

    let lines: Vec<&str> = stripped.lines().collect();
    if lines.len() <= max_lines {
        // Under line limit, but still enforce hard character limit
        if stripped.len() <= PREVIEW_HARD_LIMIT {
            return Ok(tera::Value::String(stripped));
        }
        // Find next line break after hard limit
        if let Some(pos) = stripped[PREVIEW_HARD_LIMIT..].find('\n') {
            return Ok(tera::Value::String(stripped[..PREVIEW_HARD_LIMIT + pos].to_string()));
        }
        return Ok(tera::Value::String(stripped[..PREVIEW_HARD_LIMIT].to_string()));
    }

    // Over line limit: take max_lines, then continue to next line break
    let mut result = lines[..max_lines].join("\n");

    // If there are more lines, extend to the next blank line or paragraph break
    if lines.len() > max_lines {
        // Find the next line break (empty line or end of content)
        for line in &lines[max_lines..] {
            if line.trim().is_empty() {
                break;
            }
            result.push('\n');
            result.push_str(line);
            // Check hard limit
            if result.len() >= PREVIEW_HARD_LIMIT {
                result.truncate(PREVIEW_HARD_LIMIT);
                break;
            }
        }
    }

    // Final hard limit check
    if result.len() > PREVIEW_HARD_LIMIT {
        result.truncate(PREVIEW_HARD_LIMIT);
    }

    Ok(tera::Value::String(result))
}

/// Check if text has more than N lines after stripping block quotes,
/// or exceeds 1024 characters (for showing "read more" button)
fn has_more_lines_filter(
    value: &tera::Value,
    args: &std::collections::HashMap<String, tera::Value>,
) -> tera::Result<tera::Value> {
    let s = value
        .as_str()
        .ok_or_else(|| tera::Error::msg("has_more_lines filter expects a string"))?;

    let max_lines = args
        .get("lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_PREVIEW_LINES as u64) as usize;

    // Strip block quotes first (same as preview_filter)
    let stripped = strip_block_quotes(s);

    let line_count = stripped.lines().count();
    Ok(tera::Value::Bool(line_count > max_lines || stripped.len() > PREVIEW_HARD_LIMIT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_block_quotes_simple() {
        let input = "> quoted line\nActual content";
        assert_eq!(strip_block_quotes(input), "Actual content");
    }

    #[test]
    fn test_strip_block_quotes_without_attribution() {
        // Block quote at start without attribution line
        let input = "> Hello,\n> This is quoted.\n\nActual content here.";
        assert_eq!(strip_block_quotes(input), "Actual content here.");
    }

    #[test]
    fn test_strip_block_quotes_with_attribution_no_whitespace() {
        // Attribution line directly followed by block quote (no empty line)
        let input = "On Wed, 29 Oct 2025, John Smith wrote:\n> Hello,\n> Quoted text.\n\nActual content.";
        assert_eq!(strip_block_quotes(input), "Actual content.");
    }

    #[test]
    fn test_strip_block_quotes_with_attribution_and_whitespace() {
        // Attribution line followed by empty line then block quote
        let input = "On Wed, 29 Oct 2025, John Smith wrote:\n\n> Hello,\n> Quoted text.\n\nActual content.";
        assert_eq!(strip_block_quotes(input), "Actual content.");
    }

    #[test]
    fn test_strip_block_quotes_at_end() {
        let input = "Actual content.\n\n> Quoted at end.";
        assert_eq!(strip_block_quotes(input), "Actual content.");
    }

    #[test]
    fn test_strip_block_quotes_at_both_ends() {
        let input = "> Quoted at start.\n\nActual content.\n\n> Quoted at end.";
        assert_eq!(strip_block_quotes(input), "Actual content.");
    }

    #[test]
    fn test_strip_block_quotes_preserves_middle() {
        // Quotes in the middle of content are preserved
        let input = "Start content.\n\n> Quoted in middle.\n\nEnd content.";
        assert_eq!(strip_block_quotes(input), "Start content.\n\n> Quoted in middle.\n\nEnd content.");
    }

    #[test]
    fn test_strip_block_quotes_all_quotes() {
        // When everything is quoted, result is empty
        let input = "> Only quotes\n> Nothing else";
        assert_eq!(strip_block_quotes(input), "");
    }

    #[test]
    fn test_is_quote_line_block_quote() {
        assert!(is_quote_line("> quoted"));
        assert!(is_quote_line("  > indented quote"));
        assert!(is_quote_line(">"));
    }

    #[test]
    fn test_is_quote_line_attribution() {
        assert!(is_quote_line("On Wed, 29 Oct 2025, John Smith wrote:"));
        assert!(is_quote_line("On Thu, 30 Oct 2025, Someone writes:"));
        assert!(is_quote_line("On Mon, 1 Jan 2024, Person said:"));
        assert!(is_quote_line("On Tue, 2 Feb 2024, Another says:"));
        // name <email> pattern
        assert!(is_quote_line("John Smith <john@example.com> writes:"));
        assert!(is_quote_line("Jane Doe <jane@test.org> wrote:"));
        assert!(is_quote_line("Someone <user@domain.com> said:"));
        assert!(is_quote_line("Another <another@example.net> says:"));
    }

    #[test]
    fn test_is_quote_line_not_quote() {
        assert!(!is_quote_line("Normal text"));
        assert!(!is_quote_line("On vacation"));
        assert!(!is_quote_line("Something wrote something"));
    }
}
