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


/// Resource ceilings applied before and after extraction. The caller may
/// tighten these values but cannot use them to bypass the ingestion store's
/// independent 32 MiB source-artifact ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self { max_input_bytes: 32 * 1024 * 1024, max_output_bytes: 16 * 1024 * 1024 }
    }
}

/// Auditable result metadata. Hashes bind the derived text to the exact
/// original bytes; they do not establish that the source is trustworthy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionProvenance {
    pub declared_media_type: String,
    pub extracted_media_type: String,
    pub method: String,
    pub source_sha256: String,
    pub extracted_sha256: String,
    pub source_bytes: usize,
    pub extracted_bytes: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDocument {
    pub extracted: ExtractedText,
    pub provenance: ExtractionProvenance,
}

/// Bounded dispatcher for the currently supported formats. Unsupported
/// formats fail closed instead of being guessed from their filename.
/// Extracted content is untrusted input and must never be treated as
/// instructions, verified knowledge, or permission to execute tools.
pub fn extract_document(
    bytes: &[u8],
    declared_media_type: &str,
    limits: ExtractionLimits,
) -> Result<ExtractedDocument> {
    use sha2::{Digest, Sha256};

    if bytes.is_empty() {
        return Err(anyhow!("document input is empty"));
    }
    if bytes.len() > limits.max_input_bytes {
        return Err(anyhow!("document exceeds configured input byte limit"));
    }
    if limits.max_output_bytes == 0 {
        return Err(anyhow!("configured extraction output limit must be greater than zero"));
    }

    let media_type = declared_media_type.split(';').next().unwrap_or("")
        .trim().to_ascii_lowercase();
    let (extracted, method, warnings) = match media_type.as_str() {
        "text/plain" => (
            extract_plain_text(bytes)?,
            "plain_utf8_v1".to_owned(),
            Vec::new(),
        ),
        "text/html" | "application/xhtml+xml" => (
            extract_simple_html(std::str::from_utf8(bytes)
                .map_err(|error| anyhow!("HTML input is not valid UTF-8: {error}"))?)?,
            "simple_html_v1".to_owned(),
            vec![
                "Simple HTML extraction is not a standards-complete HTML5 parser.".to_owned(),
                "HTML text is normalized; extracted offsets do not map to exact source byte ranges.".to_owned(),
                "Extracted text is untrusted data and is not sanitized HTML.".to_owned(),
            ],
        ),
        other => return Err(anyhow!("unsupported document media type: {other}")),
    };

    let extracted_bytes = extracted.text.len();
    if extracted_bytes > limits.max_output_bytes {
        return Err(anyhow!("extracted text exceeds configured output byte limit"));
    }

    Ok(ExtractedDocument {
        provenance: ExtractionProvenance {
            declared_media_type: declared_media_type.to_owned(),
            extracted_media_type: extracted.media_type.clone(),
            method,
            source_sha256: format!("{:x}", Sha256::digest(bytes)),
            extracted_sha256: format!("{:x}", Sha256::digest(extracted.text.as_bytes())),
            source_bytes: bytes.len(),
            extracted_bytes,
            warnings,
        },
        extracted,
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
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "times" => '×',
        "divide" => '÷',
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
    use sha2::Digest;

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
        assert!(result.text.contains('×'));
    }

    #[test]
    fn html_rejects_empty_or_nontext_documents() {
        assert!(extract_simple_html("<html><script>hidden</script></html>").is_err());
        assert!(extract_plain_text(&[0xff]).is_err());
    }

    #[test]
    fn bounded_dispatcher_records_hashes_and_marks_html_as_untrusted() {
        let html = b"<html><body><h1>Manual</h1><script>steal()</script><p>Voltage</p></body></html>";
        let result = extract_document(html, "text/html; charset=utf-8", ExtractionLimits::default()).unwrap();
        assert!(result.extracted.text.contains("Manual"));
        assert!(!result.extracted.text.contains("steal"));
        assert_eq!(result.provenance.source_bytes, html.len());
        assert_eq!(result.provenance.extracted_bytes, result.extracted.text.len());
        assert_eq!(result.provenance.method, "simple_html_v1");
        assert_eq!(result.provenance.warnings.len(), 3);
        assert_eq!(result.provenance.source_sha256, format!("{:x}", sha2::Sha256::digest(html)));
    }

    #[test]
    fn bounded_dispatcher_rejects_oversize_and_unsupported_inputs() {
        let limits = ExtractionLimits { max_input_bytes: 4, max_output_bytes: 8 };
        assert!(extract_document(b"12345", "text/plain", limits).is_err());
        assert!(extract_document(b"%PDF", "application/pdf", ExtractionLimits::default()).is_err());
        assert!(extract_document(b"hello", "text/plain", ExtractionLimits {
            max_input_bytes: 10, max_output_bytes: 4,
        }).is_err());
    }

}
