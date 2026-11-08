use super::Error;
use serde_json::Value;
use std::{collections::HashSet, ops::Range};

pub(super) const CONFIG_LIMIT: usize = 4 * 1024 * 1024;
pub(super) type Patch = (Vec<String>, Option<Value>);

struct Member { key: String, start: usize, value: Node }
struct Node { span: Range<usize>, members: Option<Vec<Member>> }
struct Parser<'a> { bytes: &'a [u8], at: usize }
impl Parser<'_> {
    fn ws(&mut self) { while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) { self.at += 1; } }
    fn string(&mut self) -> Result<Range<usize>, Error> {
        let start = self.at;
        if self.bytes.get(self.at) != Some(&b'"') { return Err(Error::InvalidFormat); }
        self.at += 1;
        while let Some(byte) = self.bytes.get(self.at) {
            self.at += 1;
            match byte {
                b'\\' => { self.at += 1; }
                b'"' => return Ok(start..self.at),
                _ => {}
            }
        }
        Err(Error::InvalidFormat)
    }
    fn node(&mut self, depth: usize) -> Result<Node, Error> {
        if depth > 64 { return Err(Error::Limit); }
        self.ws();
        let start = self.at;
        let mut members = None;
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1; self.ws();
                let mut entries = Vec::new(); let mut keys = HashSet::new();
                if self.bytes.get(self.at) != Some(&b'}') {
                    loop {
                        self.ws(); let member_start = self.at;
                        let key_span = self.string()?;
                        let key: String = serde_json::from_slice(&self.bytes[key_span]).map_err(|_| Error::InvalidFormat)?;
                        if !keys.insert(key.clone()) { return Err(Error::InvalidFormat); }
                        self.ws();
                        if self.bytes.get(self.at) != Some(&b':') { return Err(Error::InvalidFormat); }
                        self.at += 1;
                        let value = self.node(depth + 1)?;
                        entries.push(Member { key, start: member_start, value });
                        self.ws();
                        match self.bytes.get(self.at) {
                            Some(b',') => self.at += 1,
                            Some(b'}') => break,
                            _ => return Err(Error::InvalidFormat),
                        }
                    }
                }
                self.at += 1; members = Some(entries);
            }
            Some(b'[') => {
                self.at += 1; self.ws();
                if self.bytes.get(self.at) != Some(&b']') {
                    loop {
                        self.node(depth + 1)?; self.ws();
                        match self.bytes.get(self.at) {
                            Some(b',') => self.at += 1,
                            Some(b']') => break,
                            _ => return Err(Error::InvalidFormat),
                        }
                    }
                }
                self.at += 1;
            }
            Some(b'"') => { self.string()?; }
            Some(_) => {
                while self.bytes.get(self.at).is_some_and(|b| !b.is_ascii_whitespace() && !b",]}".contains(b)) { self.at += 1; }
                if start == self.at { return Err(Error::InvalidFormat); }
            }
            None => return Err(Error::InvalidFormat),
        }
        Ok(Node { span: start..self.at, members })
    }
}
fn json_tree(text: &str) -> Result<Node, Error> {
    if text.len() > CONFIG_LIMIT { return Err(Error::Limit); }
    let mut parser = Parser { bytes: text.as_bytes(), at: 0 };
    let node = parser.node(0)?; parser.ws();
    if parser.at != text.len() || node.members.is_none() { return Err(Error::InvalidFormat); }
    // Validate literals, escapes, numbers, whitespace and trailing commas with serde.
    serde_json::from_str::<Value>(text).map_err(|_| Error::InvalidFormat)?;
    Ok(node)
}
pub(super) fn json_read(text: &str) -> Result<Value, Error> {
    json_tree(text)?;
    serde_json::from_str(text).map_err(|_| Error::InvalidFormat)
}
fn json_edit(text: &str, path: &[String], replacement: &Option<Value>) -> Result<String, Error> {
    if path.is_empty() { return Err(Error::InvalidFormat); }
    let root = json_tree(text)?;
    let mut node = &root;
    for (index, key) in path.iter().enumerate() {
        let members = node.members.as_ref().ok_or(Error::InvalidFormat)?;
        let found = members.iter().position(|m| &m.key == key);
        if let Some(position) = found {
            let member = &members[position];
            if index + 1 != path.len() { node = &member.value; continue; }
            let mut result = text.to_owned();
            if let Some(value) = replacement {
                let encoded = serde_json::to_string(value).map_err(|_| Error::InvalidFormat)?;
                result.replace_range(member.value.span.clone(), &encoded);
            } else {
                // Remove only this member and the one necessary comma. All other
                // member/value bytes, including secrets, remain untouched.
                let range = if position + 1 < members.len() {
                    member.start..members[position + 1].start
                } else if position > 0 {
                    members[position - 1].value.span.end..member.value.span.end
                } else { member.start..member.value.span.end };
                result.replace_range(range, "");
            }
            json_tree(&result)?;
            return Ok(result);
        }
        let Some(mut value) = replacement.clone() else { return Ok(text.into()); };
        for part in path[index + 1..].iter().rev() {
            let mut object = serde_json::Map::new(); object.insert(part.clone(), value); value = Value::Object(object);
        }
        let encoded_key = serde_json::to_string(key).map_err(|_| Error::InvalidFormat)?;
        let encoded_value = serde_json::to_string(&value).map_err(|_| Error::InvalidFormat)?;
        let addition = format!("{}{}:{}", if members.is_empty() { "" } else { "," }, encoded_key, encoded_value);
        let mut result = text.to_owned();
        let insertion = members.last().map_or(node.span.end - 1, |m| m.value.span.end);
        result.insert_str(insertion, &addition);
        json_tree(&result)?;
        return Ok(result);
    }
    Err(Error::InvalidFormat)
}
fn toml_value(value: &Value) -> Result<toml_edit::Value, Error> {
    Ok(match value {
        Value::String(v) => toml_edit::Value::from(v.as_str()),
        Value::Bool(v) => toml_edit::Value::from(*v),
        Value::Number(v) => toml_edit::Value::from(v.as_i64().ok_or(Error::InvalidFormat)?),
        Value::Array(values) => {
            let mut array = toml_edit::Array::new();
            for value in values { array.push(toml_value(value)?); }
            toml_edit::Value::Array(array)
        }
        Value::Object(values) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, value) in values { table.insert(key, toml_value(value)?); }
            toml_edit::Value::InlineTable(table)
        }
        Value::Null => return Err(Error::InvalidFormat),
    })
}
fn toml_edit_at(table: &mut dyn toml_edit::TableLike, path: &[String], value: &Option<Value>) -> Result<(), Error> {
    let key = &path[0];
    if path.len() == 1 {
        if let Some(value) = value {
            let mut new_value = toml_value(value)?;
            if let Some(old) = table.get(key).and_then(toml_edit::Item::as_value) { *new_value.decor_mut() = old.decor().clone(); }
            table.insert(key, toml_edit::Item::Value(new_value));
        } else { table.remove(key); }
        return Ok(());
    }
    if !table.contains_key(key) {
        if value.is_none() { return Ok(()); }
        table.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let child = table.get_mut(key).and_then(toml_edit::Item::as_table_like_mut).ok_or(Error::InvalidFormat)?;
    toml_edit_at(child, &path[1..], value)
}
pub(super) fn apply(text: &str, target: super::Target, patches: &[Patch]) -> Result<String, Error> {
    if text.len() > CONFIG_LIMIT { return Err(Error::Limit); }
    let output = match target {
        super::Target::Claude => {
            let mut output = if text.is_empty() { "{}".into() } else { text.to_owned() };
            json_tree(&output)?;
            for (path, value) in patches { output = json_edit(&output, path, value)?; }
            output
        }
        super::Target::Codex => {
            let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
            for (path, value) in patches {
                if path.is_empty() { return Err(Error::InvalidFormat); }
                toml_edit_at(doc.as_table_mut(), path, value)?;
            }
            let output = doc.to_string();
            output.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
            output
        }
    };
    if output.len() > CONFIG_LIMIT { return Err(Error::Limit); }
    Ok(output)
}
