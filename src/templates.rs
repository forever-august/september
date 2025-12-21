use chrono::{DateTime, Utc};
use tera::Tera;

use crate::error::AppError;

/// Initialize the Tera template engine
pub fn init_templates() -> Result<Tera, AppError> {
    let mut tera = Tera::new("templates/**/*")?;

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
        .unwrap_or(50) as usize;

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
            } else if seconds < 60 {
                "just now".to_string()
            } else if seconds < 3600 {
                let mins = seconds / 60;
                if mins == 1 {
                    "1 minute ago".to_string()
                } else {
                    format!("{} minutes ago", mins)
                }
            } else if seconds < 86400 {
                let hours = seconds / 3600;
                if hours == 1 {
                    "1 hour ago".to_string()
                } else {
                    format!("{} hours ago", hours)
                }
            } else if seconds < 2592000 {
                let days = seconds / 86400;
                if days == 1 {
                    "1 day ago".to_string()
                } else {
                    format!("{} days ago", days)
                }
            } else if seconds < 31536000 {
                let months = seconds / 2592000;
                if months == 1 {
                    "1 month ago".to_string()
                } else {
                    format!("{} months ago", months)
                }
            } else {
                let years = seconds / 31536000;
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

/// Truncate text to a preview of N lines
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
        .unwrap_or(10) as usize;

    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= max_lines {
        Ok(tera::Value::String(s.to_string()))
    } else {
        let truncated = lines[..max_lines].join("\n");
        Ok(tera::Value::String(truncated))
    }
}

/// Check if text has more than N lines (for showing "read more" button)
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
        .unwrap_or(10) as usize;

    let line_count = s.lines().count();
    Ok(tera::Value::Bool(line_count > max_lines))
}
