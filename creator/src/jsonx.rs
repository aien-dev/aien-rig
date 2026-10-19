//! Pulling one JSON object out of a model reply.
//!
//! Models wrap JSON in prose or code fences, think out loud first (`<think>`),
//! leave trailing commas, keep writing after the object closes, or stop before
//! it closes (token limit). This parser handles all of those: it scans from the
//! first `{` to its matching `}` while skipping braces inside strings, and if
//! the reply was cut off it backs up to the last complete value and closes it.

use serde_json::Value;

/// Remove `<think>...</think>` blocks (and an unclosed `<think>` up to the first `{`).
pub fn strip_thinking(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find("<think>") {
        out.push_str(&rest[..p]);
        match rest[p..].find("</think>") {
            Some(e) => rest = &rest[p + e + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Find and parse the first JSON object in `reply`.
pub fn extract_object(reply: &str) -> Option<Value> {
    let text = strip_thinking(reply);
    let start = text.find('{')?;
    let body = &text[start..];

    let mut stack: Vec<char> = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    // Places where the object could be cut and closed: (byte offset, closers needed).
    let mut cut_points: Vec<(usize, Vec<char>)> = Vec::new();
    for (i, ch) in body.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop() != Some(ch) {
                    break; // mismatched: fall through to repair
                }
                if stack.is_empty() {
                    return parse_lenient(&body[..=i]);
                }
                cut_points.push((i + 1, stack.clone()));
            }
            ',' => cut_points.push((i, stack.clone())),
            _ => {}
        }
    }

    // Cut off: try closing at the end, then at each earlier cut point.
    if !in_str {
        let mut s = body.trim_end().trim_end_matches(',').to_string();
        for c in stack.iter().rev() {
            s.push(*c);
        }
        if let Some(v) = parse_lenient(&s) {
            return Some(v);
        }
    }
    for (pos, closers) in cut_points.iter().rev() {
        let mut s = body[..*pos].trim_end().trim_end_matches(',').to_string();
        for c in closers.iter().rev() {
            s.push(*c);
        }
        if let Some(v) = parse_lenient(&s) {
            return Some(v);
        }
    }
    None
}

fn parse_lenient(s: &str) -> Option<Value> {
    serde_json::from_str(s).ok().or_else(|| serde_json::from_str(&remove_trailing_commas(s)).ok())
}

/// Drop commas that directly precede `}` or `]` (outside strings).
fn remove_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut esc = false;
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        }
        if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_object() {
        let v = extract_object(r#"{"a": 1}"#).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn prose_fences_and_thinking() {
        let r = "<think>maybe {\"wrong\": true}</think>Sure! Here it is:\n```json\n{\"titles\": [\"A {b}\", \"C\"]}\n```\nHope that helps {x}";
        let v = extract_object(r).unwrap();
        assert_eq!(v["titles"][0], "A {b}");
        assert!(v.get("wrong").is_none());
    }

    #[test]
    fn trailing_commas() {
        let v = extract_object("{\"a\": [1, 2,], \"b\": \"x,]\",}").unwrap();
        assert_eq!(v["a"][1], 2);
        assert_eq!(v["b"], "x,]");
    }

    #[test]
    fn keeps_writing_after_close() {
        let v = extract_object("{\"a\": \"}\"} and then {\"b\": 2}").unwrap();
        assert_eq!(v["a"], "}");
    }

    #[test]
    fn cut_off_reply_keeps_complete_parts() {
        let r = "{\"titles\": [\"one\", \"two\"], \"tags\": [\"x\", \"y\"], \"description\": \"half a sent";
        let v = extract_object(r).unwrap();
        assert_eq!(v["titles"][1], "two");
        assert_eq!(v["tags"][1], "y");
        assert!(v.get("description").is_none());

        let r = "{\"shorts\": [{\"start\": \"01:00\", \"end\": \"01:30\"}, {\"start\": \"02:";
        let v = extract_object(r).unwrap();
        assert_eq!(v["shorts"].as_array().unwrap().len(), 1);

        let r = "{\"a\": [1, 2";
        assert_eq!(extract_object(r).unwrap()["a"][1], 2);
    }

    #[test]
    fn escaped_quotes() {
        let v = extract_object(r#"{"hook": "he said \"go{\" loud"}"#).unwrap();
        assert_eq!(v["hook"], "he said \"go{\" loud");
    }

    #[test]
    fn nothing_there() {
        assert!(extract_object("no json here").is_none());
        assert!(extract_object("").is_none());
    }
}
