use super::event_size;
use crate::session::SessionView;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

pub(super) const MAX_PAGES: usize = 64;
pub(super) const MAX_EVENTS: usize = 2000;
pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
pub(super) const TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);
const PAGE_SIZE: usize = 128;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct Page {
    pub method: &'static str,
    pub turn: Option<String>,
    pub cursor: Option<String>,
    pub descending: bool,
}
impl Page {
    pub fn params(&self, thread: &str) -> Value {
        let limit = if self.method == "thread/items/list" { 1 } else { PAGE_SIZE };
        let mut params = json!({"threadId":thread,"cursor":self.cursor,"limit":limit,"sortDirection":if self.descending { "desc" } else { "asc" }});
        if self.method == "thread/turns/list" { params["itemsView"] = json!("notLoaded"); }
        if let Some(turn) = &self.turn { params["turnId"] = json!(turn); }
        params
    }
}

struct ItemIdentity {
    seq: u64,
    full: bool,
    bytes: usize,
}

// Only ordering/identity metadata lives here. SessionView remains the sole event store.
#[derive(Default)]
pub(super) struct History {
    turns: Vec<String>,
    items: HashMap<String, Vec<String>>,
    identities: HashMap<String, HashMap<String, ItemIdentity>>,
    event_count: usize,
    queued: HashSet<Page>,
    next_edges: HashMap<Page, Page>,
    pub pages: VecDeque<Page>,
    bytes: usize,
    pub exhausted: bool,
}
impl History {
    pub fn seed(&mut self, view: &mut SessionView, result: &Value) {
        match result.pointer("/thread/turns").and_then(Value::as_array) {
            Some(turns) => self.turns(view, turns, false),
            None => view.history_truncated = true,
        }
        if let Some(initial) = result.get("initialTurnsPage").filter(|v| !v.is_null()) {
            let page = Page { method: "thread/turns/list", turn: None, cursor: None, descending: true };
            self.page(view, &page, initial);
        }
        for (field, method) in [("turnsBackwardsCursor", "thread/turns/list"), ("itemsBackwardsCursor", "thread/items/list")] {
            self.cursor(view, result, field, Page { method, turn: None, cursor: None, descending: true }, false, false);
        }
    }
    fn queue(&mut self, page: Page) {
        if self.queued.insert(page.clone()) { self.pages.push_back(page); }
    }
    fn cursor(&mut self, view: &mut SessionView, result: &Value, field: &str, mut page: Page, required: bool, continuation: bool) {
        match result.get(field) {
            Some(Value::Null) => {},
            None if !required => {},
            Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 4096 => {
                let source = continuation.then(|| page.clone());
                page.cursor = Some(cursor.clone());
                if let Some(source) = source {
                    let mut next = &page;
                    for _ in 0..=self.next_edges.len() {
                        if next == &source {
                            view.history_truncated = true;
                            return;
                        }
                        let Some(edge) = self.next_edges.get(next) else { break; };
                        next = edge;
                    }
                    // Separate overlapping resume anchors may converge on an
                    // already visited page. Only a continuation cycle is incomplete.
                    self.next_edges.insert(source, page.clone());
                }
                self.queue(page);
            }
            _ => view.history_truncated = true,
        }
    }
    pub fn page(&mut self, view: &mut SessionView, page: &Page, result: &Value) {
        if let Some(data) = result.get("data").and_then(Value::as_array) {
            if page.method == "thread/turns/list" { self.turns(view, data, page.descending); }
            else { self.entries(view, data, page); }
        } else { view.history_truncated = true; }
        // nextCursor continues in the requested direction. backwardsCursor reverses it
        // and includes the anchor again; duplicate reverse anchors are expected.
        self.cursor(view, result, "nextCursor", page.clone(), true, true);
        let reverse = Page { descending: !page.descending, ..page.clone() };
        self.cursor(view, result, "backwardsCursor", reverse, true, false);
    }
    fn turns(&mut self, view: &mut SessionView, turns: &[Value], descending: bool) {
        let mut order = Vec::new();
        let indices = (0..turns.len()).map(|index| if descending { turns.len() - 1 - index } else { index });
        for index in indices {
            if self.exhausted { break; }
            let turn = &turns[index];
            let Some(id) = identity(turn, "id") else { view.history_truncated = true; continue; };
            if !self.items.contains_key(id) && self.items.len() >= MAX_EVENTS {
                view.history_truncated = true;
                self.exhausted = true;
                break;
            }
            self.items.entry(id.into()).or_default();
            if !order.iter().any(|old| old == id) { order.push(id.to_owned()); }
            let Some(items) = turn.get("items").and_then(Value::as_array) else { view.history_truncated = true; continue; };
            let full = turn.get("itemsView").and_then(Value::as_str).is_none_or(|view| view == "full");
            self.turn_items(view, id, items.iter().map(|item| (item, None)), descending, full);
            match turn.get("itemsView").and_then(Value::as_str) {
                None | Some("full") => {},
                Some("summary" | "notLoaded") => self.queue(Page { method: "thread/items/list", turn: Some(id.into()), cursor: None, descending: false }),
                _ => view.history_truncated = true,
            }
        }
        merge(&mut self.turns, order, descending);
    }
    fn entries(&mut self, view: &mut SessionView, entries: &[Value], page: &Page) {
        let mut order = Vec::new();
        let indices = (0..entries.len()).map(|index| if page.descending { entries.len() - 1 - index } else { index });
        // Entries are globally ordered. The items inside a Turn, in contrast, are
        // always chronological even when the containing turns page is descending.
        let mut per_turn: Vec<(String, Vec<(&Value, Option<&Value>)>)> = Vec::new();
        for index in indices {
            let entry = &entries[index];
            let Some(turn) = identity(entry, "turnId") else { view.history_truncated = true; continue; };
            if page.turn.as_deref().is_some_and(|expected| expected != turn) { view.history_truncated = true; continue; }
            let Some(item) = entry.get("item").filter(|v| v.is_object()) else { view.history_truncated = true; continue; };
            if !self.items.contains_key(turn) && self.items.len() >= MAX_EVENTS {
                view.history_truncated = true;
                self.exhausted = true;
                break;
            }
            self.items.entry(turn.into()).or_default();
            if !order.iter().any(|old| old == turn) { order.push(turn.to_owned()); }
            if let Some((_, items)) = per_turn.iter_mut().find(|(id, _)| id == turn) { items.push((item, Some(entry))); }
            else { per_turn.push((turn.into(), vec![(item, Some(entry))])); }
        }
        for (turn, items) in per_turn { self.turn_items(view, &turn, items.into_iter(), page.descending, true); }
        merge(&mut self.turns, order, page.descending);
    }
    fn turn_items<'a>(&mut self, view: &mut SessionView, turn: &str, items: impl Iterator<Item = (&'a Value, Option<&'a Value>)>, prepend: bool, full: bool) {
        let mut order = Vec::new();
        let identities = self.identities.entry(turn.into()).or_default();
        for (item, entry) in items {
            let Some(id) = identity(item, "id") else { view.history_truncated = true; continue; };
            let Some(kind) = super::safe_type(item.get("type").and_then(Value::as_str)) else { view.history_truncated = true; continue; };
            let previous = identities.get(id);
            if previous.is_none() || (full && previous.is_some_and(|previous| !previous.full)) {
                // Match ThreadItemEntry -> ThreadItem in the pinned protocol:
                // entry timestamps fill only missing/null item timestamps.
                let overlays = ["startedAtMs", "completedAtMs"].map(|field| {
                    entry.and_then(|entry| entry.get(field)).filter(|value| value.as_i64().is_some() && item.get(field).is_none_or(Value::is_null)).map(|value| (field, value))
                });
                let text = super::item_text(item);
                let mut bytes = event_size(item) + text.as_ref().map_or(0, String::len);
                for (field, value) in overlays.iter().flatten() {
                    bytes += event_size(value);
                    if let Some(old) = item.get(*field) { bytes -= event_size(old); }
                    else { bytes += field.len(); }
                }
                let old_bytes = previous.map_or(0, |previous| previous.bytes);
                if (previous.is_none() && self.event_count >= MAX_EVENTS) || self.bytes - old_bytes + bytes > MAX_BYTES {
                    view.history_truncated = true;
                    self.exhausted = true;
                    break;
                }
                let mut data = super::item_data(item);
                for (field, value) in overlays.into_iter().flatten() { data[field] = value.clone(); }
                if let Some(previous) = previous {
                    let seq = previous.seq;
                    if !view.replace_history_event(seq, kind, text, data) {
                        view.history_truncated = true;
                        self.exhausted = true;
                        break;
                    }
                    identities.insert(id.into(), ItemIdentity { seq, full, bytes });
                } else {
                    view.push(kind, text, data);
                    identities.insert(id.into(), ItemIdentity { seq: view.events.last().unwrap().seq, full, bytes });
                    self.event_count += 1;
                }
                self.bytes = self.bytes - old_bytes + bytes;
            }
            if !order.iter().any(|old| old == id) { order.push(id.into()); }
        }
        merge(self.items.entry(turn.into()).or_default(), order, prepend);
    }
    pub fn finish(self, view: &mut SessionView) {
        let mut ranks = HashMap::new();
        for (turn_rank, turn) in self.turns.iter().enumerate() {
            if let Some(items) = self.items.get(turn) {
                for (item_rank, item) in items.iter().enumerate() {
                    if let Some(identity) = self.identities.get(turn).and_then(|items| items.get(item)) { ranks.insert(identity.seq, (turn_rank, item_rank)); }
                }
            }
        }
        view.events.sort_unstable_by_key(|event| (ranks.get(&event.seq).copied().unwrap_or((usize::MAX, usize::MAX)), event.seq));
        for (index, event) in view.events.iter_mut().enumerate() { event.seq = index as u64 + 1; }
    }
}
fn identity<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= 4096)
}

// Merge chronological identities around inclusive overlapping anchors, never text.
fn merge(existing: &mut Vec<String>, incoming: Vec<String>, prepend: bool) {
    let mut position = if prepend { 0 } else { existing.len() };
    for (index, id) in incoming.iter().enumerate() {
        if let Some(found) = existing.iter().position(|old| old == id) { position = found + 1; continue; }
        if let Some(next) = incoming[index + 1..].iter().find_map(|next| existing.iter().position(|old| old == next)) { position = next; }
        existing.insert(position, id.clone());
        position += 1;
    }
}
