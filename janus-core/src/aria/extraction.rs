//! Lightweight text normalization helpers for Aria ingestion.
//!
//! These helpers do not claim to be standards-complete document parsers.
//! PDF, DOCX, OCR, and full HTML5 extraction should be supplied by audited
//! format adapters at the host boundary.

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedText {
    pub media_type: String,
    pub text: String,
}

/// Plain UTF-8 text is preserved byte-for-byte after validation.
pub fn extract_plain_text(bytes: &[u8]) -> Result<ExtractedText> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| anyhow!("plain-text input is not valid UTF-8: {error}"))?;
    if text.trim().is_empty() {
        return Err(anyhow!("document contains no extractable text"));
    }
    Ok(ExtractedText {
        media_type: "text/plain; charset=utf-8".into(),
        text: text.to_owned(),
    })
}

/// Conservative HTML text extraction for simple exported pages.
/// Strips tags, ignores script/style contents, decodes common and numeric
/// entities, and inserts line breaks at common block boundaries. This is not
/// a complete HTML5 parser and must not be used as a security sanitizer.
pub fn extract_simple_html(input: &str) -> Result<ExtractedText> {
    if input.trim().is_empty() {
        return Err(anyhow!("HTML input is empty"));
    }
    let mut output = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    let mut skip: Option<String> = None;
    let mut entity = String::new();
    let mut in_entity = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_tag {
            if ch == '>' {
                let raw = tag.trim();
                let closing = raw.starts_with('/');
                let name = raw.trim_start_matches('/').split_whitespace().next()
                    .unwrap_or("").trim_end_matches('/').to_ascii_lowercase();
                if closing && skip.as_deref() == Some(name.as_str()) {
                    skip = None;
                } else if !closing && (name == "script" || name == "style") {
                    skip = Some(name.clone());
                } else if skip.is_none() && matches!(name.as_str(),
                    "p"|"div"|"br"|"li"|"tr"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|
                    "section"|"article"|"header"|"footer"|"blockquote"|"pre") {
                    push_newline(&mut output);
                }
                tag.clear();
                in_tag = false;
            } else if tag.len() < 256 {
                tag.push(ch);
            }
            continue;
        }

        if ch == '<' && !in_entity {
            if !skip.is_some() && !output.ends_with(' ') && !output.ends_with('\n') {
                output.push(' ');
            }
            in_tag = true;
            tag.clear();
            continue;
        }
        if skip.is_some() {
            continue;
        }
        if in_entity {
            if ch == ';' {
                output.push(decode_entity(&entity));
                entity.clear();
                in_entity = false;
            } else if entity.len() < 16 && (ch.is_ascii_alphanumeric() || ch == '#') {
                entity.push(ch);
            } else {
                output.push('&');
                output.push_str(&entity);
                entity.clear();
                in_entity = false;
                output.push(ch);
            }
        } else if ch == '&' {
            in_entity = true;
        } else {
            output.push(ch);
        }
    }
    if in_entity {
        output.push('&');
        output.push_str(&entity);
    }
    let normalized = output.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Err(anyhow!("HTML document contains no extractable text"));
    }
    Ok(ExtractedText {
        media_type: "text/html; extracted-utf8".into(),
        text: normalized,
    })
}

fn push_newline(output: &mut String) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
}

fn decode_entity(entity: &str) -> char {
    match entity {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        value if value.starts_with("#x") || value.starts_with("#X") =>
            u32::from_str_radix(&value[2..],16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'),
        value if value.starts_with('#') =>
            value[1..].parse::<u32>().ok().and_then(char::from_u32).unwrap_or('\u{fffd}'),
        _ => '\u{fffd}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_preserved() {
        let text="A\nB\né";
        assert_eq!(extract_plain_text(text.as_bytes()).unwrap().text,text);
    }

    #[test]
    fn html_removes_markup_scripts_and_decodes_entities() {
        let result=extract_simple_html(
            "<h1>Voltage &amp; Current</h1><script>secret()</script><p>V = I &times; R</p>"
        ).unwrap();
        assert!(result.text.contains("Voltage & Current"));
        assert!(!result.text.contains("secret"));
        assert!(result.text.contains("V = I"));
    }

    #[test]
    fn html_rejects_empty_or_nontext_documents() {
        assert!(extract_simple_html("<html><script>hidden</script></html>").is_err());
        assert!(extract_plain_text(&[0xff]).is_err());
    }
}
