//! Native-view presentation for the Markdown constructs used by the user guide.
//! RTF is output data only: source text is always escaped before insertion.
use std::fmt::Write;

/// Short Help-page copy, separate from the complete user guide.
pub const INTRODUCTION: [&str; 5] = [
    "Typing lets you move the caret quickly with Vim and Emacs style controls and perform convenient actions through custom key mappings wherever you type.",
    "For example, with the Default profile:",
    "    • Vim style: Tap “Caps Lock” to enter command mode, then press “I / J / K / L” to move up / left / down / right.",
    "    • Emacs style: Hold “Caps Lock” and press “I / J / K / L” to move up / left / down / right directly.",
    "    • Key mapping: “Alt+C” → “Ctrl+C” does copy operation. “Alt+H” → “Ctrl+Left” moves cursor to the beginning of a word.",
];

pub fn introduction_text() -> String {
    format!(
        "{}\r\n\r\n{}",
        INTRODUCTION[0],
        INTRODUCTION[1..].join("\r\n")
    )
}

fn text(out: &mut String, value: &str) {
    for unit in value.encode_utf16() {
        match unit {
            92 | 123 | 125 => {
                out.push('\\');
                out.push(char::from_u32(unit as u32).unwrap());
            }
            9 => out.push_str("\\tab "),
            10 | 13 => out.push(' '),
            32..=126 => out.push(char::from_u32(unit as u32).unwrap()),
            _ => {
                let _ = write!(out, "\\u{}?", unit as i16);
            }
        }
    }
}

fn inline(out: &mut String, mut source: &str) {
    while !source.is_empty() {
        if let Some(tail) = source.strip_prefix('\\')
            && let Some(character) = tail.chars().next()
            && "\\`*_[]{}".contains(character)
        {
            text(out, &tail[..character.len_utf8()]);
            source = &tail[character.len_utf8()..];
            continue;
        }
        let mut formatted = false;
        for (marker, style) in [("**", "\\b "), ("`", "\\f1 "), ("*", "\\i ")] {
            if marker == "*" && source.starts_with("**") {
                continue;
            }
            if let Some(tail) = source.strip_prefix(marker)
                && let Some(end) = tail.find(marker)
            {
                out.push('{');
                out.push_str(style);
                if marker == "`" {
                    text(out, &tail[..end]);
                } else {
                    inline(out, &tail[..end]);
                }
                out.push('}');
                source = &tail[end + marker.len()..];
                formatted = true;
                break;
            }
        }
        if formatted {
            continue;
        }
        if let Some(tail) = source.strip_prefix('[')
            && let Some(end) = tail.find("](")
            && let Some(close) = tail[end + 2..].find(')')
        {
            inline(out, &tail[..end]);
            out.push_str(" (");
            text(out, &tail[end + 2..end + 2 + close]);
            out.push(')');
            source = &tail[end + 3 + close..];
            continue;
        }
        let character = source.chars().next().unwrap();
        text(out, &source[..character.len_utf8()]);
        source = &source[character.len_utf8()..];
    }
}

fn paragraph(out: &mut String, pending: &mut Vec<&str>) {
    if pending.is_empty() {
        return;
    }
    out.push_str("{\\pard\\sa120 ");
    inline(out, &pending.join(" "));
    out.push_str("\\par}\n");
    pending.clear();
}

fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

fn delimiter(line: &str) -> bool {
    let cells = cells(line);
    cells.len() > 1
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':');
            cell.len() >= 3 && cell.chars().all(|c| c == '-')
        })
}

/// Format headings, paragraphs, lists, fenced code, tables and inline styles.
/// The width is in twips; font size is in half-points, independent of Win32.
pub fn markdown_rtf(source: &str, width: i32, font_size: i32) -> String {
    let width = width.max(1);
    let font_size = font_size.max(8);
    let mut out = format!(
        "{{\\rtf1\\ansi\\ansicpg1252\\uc1\\deff0{{\\fonttbl{{\\f0 Segoe UI;}}{{\\f1 Consolas;}}}}\\viewkind4\\pard\\f0\\fs{font_size} "
    );
    let lines: Vec<_> = source.lines().collect();
    let mut pending = Vec::new();
    let mut index = 0;
    let mut code = false;
    while index < lines.len() {
        let original = lines[index];
        let line = original.trim();
        if line.starts_with("```") {
            paragraph(&mut out, &mut pending);
            code = !code;
        } else if code {
            out.push_str("{\\pard\\f1\\sa0 ");
            text(&mut out, original);
            out.push_str("\\par}\n");
        } else if line.is_empty() {
            paragraph(&mut out, &mut pending);
        } else if line.starts_with('|') && index + 1 < lines.len() && delimiter(lines[index + 1]) {
            paragraph(&mut out, &mut pending);
            let count = cells(line).len();
            let mut header = true;
            while index < lines.len() && lines[index].trim().starts_with('|') {
                if !delimiter(lines[index]) {
                    let _ = write!(
                        out,
                        "{{\\trowd\\trgaph80\\trleft0\\trautofit0\\trftsWidth3\\trwWidth{width}"
                    );
                    for column in 1..=count {
                        let edge = if count == 2 && column == 1 {
                            width / 3
                        } else {
                            width * column as i32 / count as i32
                        };
                        let _ = write!(out, "\\cellx{edge}");
                    }
                    let row = cells(lines[index]);
                    for column in 0..count {
                        out.push_str("\\pard\\intbl\\sa60 ");
                        if header {
                            out.push_str("{\\b ");
                        }
                        inline(&mut out, row.get(column).copied().unwrap_or(""));
                        if header {
                            out.push('}');
                        }
                        out.push_str("\\cell ");
                    }
                    out.push_str("\\row}\n");
                    header = false;
                }
                index += 1;
            }
            out.push_str("{\\pard\\sa80\\par}\n");
            continue;
        } else {
            let level = line.bytes().take_while(|&c| c == b'#').count();
            if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
                paragraph(&mut out, &mut pending);
                let size = font_size
                    * match level {
                        1 => 16,
                        2 => 13,
                        _ => 11,
                    }
                    / 10;
                let _ = write!(out, "{{\\pard\\sb180\\sa100\\b\\fs{size} ");
                inline(&mut out, line[level + 1..].trim_end_matches('#').trim());
                out.push_str("\\par}\n");
            } else if let Some(body) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                paragraph(&mut out, &mut pending);
                out.push_str("{\\pard\\li360\\fi-240\\sa60 ");
                text(&mut out, "• ");
                inline(&mut out, body);
                out.push_str("\\par}\n");
            } else if let Some((number, body)) = line.split_once(' ')
                && number
                    .strip_suffix('.')
                    .or_else(|| number.strip_suffix(')'))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
            {
                paragraph(&mut out, &mut pending);
                out.push_str("{\\pard\\li360\\fi-240\\sa60 ");
                text(&mut out, number);
                out.push(' ');
                inline(&mut out, body);
                out.push_str("\\par}\n");
            } else {
                pending.push(original.trim());
            }
        }
        index += 1;
    }
    paragraph(&mut out, &mut pending);
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_cannot_insert_rtf_controls_and_unicode_roundtrips_as_utf16() {
        let rtf = markdown_rtf("```\n{\\rtf1\\object} 中文 😀\n```", 6000, 22);
        assert!(rtf.contains("\\{\\\\rtf1\\\\object\\}"));
        assert!(rtf.contains("\\u20013?\\u25991?"));
        assert!(rtf.contains("\\u-10179?\\u-8704?"));
        assert!(!rtf.contains("{\\object"));
    }
    #[test]
    fn guide_structures_have_styles_and_tables_reflow_to_the_given_width() {
        let source = "# Guide\n\nHello **bold** and `code`.\n\n1. First\n- Second\n\n| Key | Action |\n| --- | --- |\n| **J** | Left |\n";
        let rtf = markdown_rtf(source, 6000, 22);
        assert!(rtf.contains("\\b\\fs35 Guide"));
        assert!(rtf.contains("{\\b bold}") && rtf.contains("{\\f1 code}"));
        assert!(rtf.contains("1. First") && rtf.contains("\\u8226? Second"));
        assert!(rtf.contains("\\cellx2000\\cellx6000"));
        assert!(!rtf.contains("---") && !rtf.contains("**"));
        assert!(markdown_rtf(source, 3000, 30).contains("\\cellx1000\\cellx3000"));
    }
    #[test]
    fn links_escapes_and_unclosed_markers_remain_readable() {
        let rtf = markdown_rtf(
            "[Guide](https://example.com) \\*literal\\* **unfinished",
            6000,
            22,
        );
        assert!(rtf.contains("Guide (https://example.com) *literal* **unfinished"));
    }
}
