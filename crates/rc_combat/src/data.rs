//! Reads our settings files: a small part of the TOML format, enough for
//! weapon files, with no extra libraries.
//!
//! ```toml
//! # a comment
//! name = "DC-17m Blaster"     # text in double quotes
//! [fire]                      # a section: names below are "fire.<name>"
//! shot_delay = 0.1            # a number
//! automatic = true            # true or false
//! sounds = ["a", "b"]         # a list of texts or of numbers
//! ```
//!
//! Every mistake (a line that doesn't read, an unknown name, a value of the
//! wrong kind) is an error naming the line, so a typo in a mod is reported
//! instead of silently ignored.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f32),
    Bool(bool),
    Text(String),
    List(Vec<Value>),
}

/// One value and the line it came from.
#[derive(Debug, Clone)]
struct Entry {
    value: Value,
    line: usize,
    used: bool,
}

/// A parsed file. Names are `section.name`, or just `name` before the
/// first section.
#[derive(Debug, Default)]
pub struct Table {
    entries: BTreeMap<String, Entry>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Table, String> {
        let mut table = Table::default();
        let mut section = String::new();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| format!("line {n}: a section needs a closing `]`"))?;
                section = name.trim().to_string();
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected `name = value`, got `{line}`"))?;
            let key = key.trim();
            let full = if section.is_empty() {
                key.to_string()
            } else {
                format!("{section}.{key}")
            };
            let value = parse_value(value.trim()).map_err(|e| format!("line {n}: {e}"))?;
            if table.entries.contains_key(&full) {
                return Err(format!("line {n}: `{full}` is set twice"));
            }
            table.entries.insert(
                full,
                Entry {
                    value,
                    line: n,
                    used: false,
                },
            );
        }
        Ok(table)
    }

    fn take(&mut self, name: &str) -> Option<(Value, usize)> {
        self.entries.get_mut(name).map(|e| {
            e.used = true;
            (e.value.clone(), e.line)
        })
    }

    /// A number, or `None` if the file doesn't set it.
    pub fn number(&mut self, name: &str) -> Result<Option<f32>, String> {
        match self.take(name) {
            None => Ok(None),
            Some((Value::Number(v), _)) => Ok(Some(v)),
            Some((_, line)) => Err(format!("line {line}: `{name}` should be a number")),
        }
    }

    pub fn boolean(&mut self, name: &str) -> Result<Option<bool>, String> {
        match self.take(name) {
            None => Ok(None),
            Some((Value::Bool(v), _)) => Ok(Some(v)),
            Some((_, line)) => Err(format!("line {line}: `{name}` should be true or false")),
        }
    }

    pub fn text(&mut self, name: &str) -> Result<Option<String>, String> {
        match self.take(name) {
            None => Ok(None),
            Some((Value::Text(v), _)) => Ok(Some(v)),
            Some((_, line)) => Err(format!("line {line}: `{name}` should be text in quotes")),
        }
    }

    /// A list of texts. A single text counts as a list of one.
    pub fn texts(&mut self, name: &str) -> Result<Option<Vec<String>>, String> {
        let bad = |line| format!("line {line}: `{name}` should be a list of texts in quotes");
        match self.take(name) {
            None => Ok(None),
            Some((Value::Text(v), _)) => Ok(Some(vec![v])),
            Some((Value::List(items), line)) => items
                .into_iter()
                .map(|v| match v {
                    Value::Text(t) => Ok(t),
                    _ => Err(bad(line)),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some((_, line)) => Err(bad(line)),
        }
    }

    /// A list of exactly three numbers, like a position.
    pub fn vec3(&mut self, name: &str) -> Result<Option<[f32; 3]>, String> {
        let bad = |line| format!("line {line}: `{name}` should be three numbers, like [1, 2, 3]");
        match self.take(name) {
            None => Ok(None),
            Some((Value::List(items), line)) => {
                let nums: Vec<f32> = items
                    .iter()
                    .map(|v| match v {
                        Value::Number(x) => Ok(*x),
                        _ => Err(bad(line)),
                    })
                    .collect::<Result<_, _>>()?;
                <[f32; 3]>::try_from(nums).map(Some).map_err(|_| bad(line))
            }
            Some((_, line)) => Err(bad(line)),
        }
    }

    /// Fails if the file set anything nobody asked for (a misspelt name).
    pub fn finish(self) -> Result<(), String> {
        match self.entries.iter().find(|(_, e)| !e.used) {
            Some((name, e)) => Err(format!("line {}: unknown setting `{name}`", e.line)),
            None => Ok(()),
        }
    }
}

/// Everything before a `#` that isn't inside quotes.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_value(s: &str) -> Result<Value, String> {
    if let Some(inner) = s.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or_else(|| "a list needs a closing `]`".to_string())?;
        let items = split_list(inner)
            .into_iter()
            .map(|p| parse_value(p.trim()))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Value::List(items));
    }
    if let Some(inner) = s.strip_prefix('"') {
        let inner = inner
            .strip_suffix('"')
            .ok_or_else(|| "text needs a closing `\"`".to_string())?;
        if inner.contains('"') {
            return Err("text can't contain `\"`".to_string());
        }
        return Ok(Value::Text(inner.to_string()));
    }
    match s {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    s.parse::<f32>()
        .map(Value::Number)
        .map_err(|_| format!("`{s}` is not a number, true/false, text in quotes or a list"))
}

/// Split a list's inside on commas that aren't inside quotes. An empty list
/// gives no items.
fn split_list(s: &str) -> Vec<&str> {
    if s.trim().is_empty() {
        return vec![];
    }
    let mut out = vec![];
    let mut start = 0;
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = &s[start..];
    if !last.trim().is_empty() {
        out.push(last);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_value() {
        let text = "name = \"DC-17m # rifle\"  # comment\n[fire]\nshot_delay = 0.1\nauto = true\n\
                    sounds = [\"a\", \"b\"]\nmuzzle = [127.4, 30, -10]\n";
        let mut t = Table::parse(text).unwrap();
        assert_eq!(t.text("name").unwrap().unwrap(), "DC-17m # rifle");
        assert_eq!(t.number("fire.shot_delay").unwrap(), Some(0.1));
        assert_eq!(t.boolean("fire.auto").unwrap(), Some(true));
        assert_eq!(t.texts("fire.sounds").unwrap().unwrap(), vec!["a", "b"]);
        assert_eq!(t.vec3("fire.muzzle").unwrap(), Some([127.4, 30.0, -10.0]));
        assert_eq!(t.number("fire.missing").unwrap(), None);
        t.finish().unwrap();
    }

    #[test]
    fn mistakes_name_the_line() {
        let mut t = Table::parse("[fire]\nshot_dleay = 0.1\n").unwrap();
        assert_eq!(t.number("fire.shot_delay").unwrap(), None);
        assert_eq!(
            t.finish().unwrap_err(),
            "line 2: unknown setting `fire.shot_dleay`"
        );
        let e = Table::parse("a = fast").unwrap_err();
        assert!(e.starts_with("line 1: `fast` is not a number"), "{e}");
        let mut t = Table::parse("\na = \"x\"").unwrap();
        assert_eq!(t.number("a").unwrap_err(), "line 2: `a` should be a number");
    }
}
