//! Minimal parser for Valve's text KeyValues format (libraryfolders.vdf,
//! appmanifest_*.acf). Pure logic.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Str(String),
    Obj(Obj),
}

pub type Obj = BTreeMap<String, Vec<Node>>;

impl Node {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&Obj> {
        match self {
            Node::Obj(o) => Some(o),
            _ => None,
        }
    }
}

/// Case-insensitive lookup of the first child with this key.
pub fn get<'a>(o: &'a Obj, key: &str) -> Option<&'a Node> {
    o.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v.first())
}

pub fn get_str<'a>(o: &'a Obj, key: &str) -> Option<&'a str> {
    get(o, key).and_then(|n| n.as_str())
}

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokenize(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '"' => {
                let mut buf = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '\\' => match it.next() {
                            Some('n') => buf.push('\n'),
                            Some('t') => buf.push('\t'),
                            Some(other) => buf.push(other),
                            None => {}
                        },
                        '"' => break,
                        _ => buf.push(c),
                    }
                }
                out.push(Tok::Str(buf));
            }
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            _ => {
                // unquoted token
                let mut buf = String::from(c);
                while let Some(&n) = it.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    buf.push(n);
                    it.next();
                }
                out.push(Tok::Str(buf));
            }
        }
    }
    out
}

fn parse_obj(toks: &[Tok], pos: &mut usize) -> Obj {
    let mut obj = Obj::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Close => {
                *pos += 1;
                return obj;
            }
            Tok::Open => {
                *pos += 1; // stray brace, skip
            }
            Tok::Str(k) => {
                let key = k.clone();
                *pos += 1;
                match toks.get(*pos) {
                    Some(Tok::Str(v)) => {
                        obj.entry(key).or_default().push(Node::Str(v.clone()));
                        *pos += 1;
                    }
                    Some(Tok::Open) => {
                        *pos += 1;
                        let child = parse_obj(toks, pos);
                        obj.entry(key).or_default().push(Node::Obj(child));
                    }
                    _ => {}
                }
            }
        }
    }
    obj
}

pub fn parse(s: &str) -> Obj {
    let toks = tokenize(s);
    let mut pos = 0;
    parse_obj(&toks, &mut pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_and_escapes() {
        let s = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "apps" { "730" "123" "570" "456" }
    }
    // comment
    "1" { "path" "D:\\SteamLibrary" }
}"#;
        let o = parse(s);
        let lf = get(&o, "LibraryFolders").unwrap().as_obj().unwrap();
        let zero = get(lf, "0").unwrap().as_obj().unwrap();
        assert_eq!(get_str(zero, "path"), Some(r"C:\Program Files (x86)\Steam"));
        let apps = get(zero, "apps").unwrap().as_obj().unwrap();
        assert_eq!(get_str(apps, "570"), Some("456"));
        let one = get(lf, "1").unwrap().as_obj().unwrap();
        assert_eq!(get_str(one, "path"), Some(r"D:\SteamLibrary"));
    }
}
