//! A minimal, path-based patch engine.
//!
//! Every conversion step in the bridge is expressed as a [`Patch`] with a
//! stable `code` (feature and action), a JSON-pointer-ish `path`, and both the
//! old and new value. Two things fall out of that:
//!
//! * A conversion report can list precisely which lines changed and why, with
//!   a machine-readable code (`data.recipe.ingredient.object_to_string`).
//! * A reversed report shows what a human would need to undo by hand, which is
//!   what "never silently discard" requires for anything that did not port.
//!
//! Paths use `a/b/0/c` with `~0`/`~1` unescaping, like RFC 6901 without the
//! leading slash.

use crate::{Object, Value};

/// One edit to a JSON document.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    /// Stable identifier, `area.thing.action`.
    pub code: &'static str,
    /// Path to the edited node.
    pub path: String,
    pub before: Node,
    pub after: Node,
    /// Why the edit was needed, in words a mod author can act on.
    pub note: String,
}

/// Presence state of a node: absent is distinct from `null`.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Absent,
    Present(Value),
}

impl Node {
    pub fn absent() -> Self {
        Node::Absent
    }

    pub fn of(value: &Value) -> Self {
        Node::Present(value.clone())
    }

    pub fn is_absent(&self) -> bool {
        matches!(self, Node::Absent)
    }

    /// Short display form for reports, truncated so a report stays readable.
    pub fn display(&self, max: usize) -> String {
        let text = match self {
            Node::Absent => return "<absent>".to_owned(),
            Node::Present(v) => crate::to_string(v),
        };
        if text.chars().count() <= max {
            text
        } else {
            let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
            out.push('…');
            out
        }
    }
}

/// Accumulates patches while a converter edits a document.
#[derive(Debug, Default)]
pub struct Editor {
    patches: Vec<Patch>,
    /// While non-zero, edits are made without recording. Used for normalising
    /// rewrites (whitespace, key order) that are not worth reporting.
    quiet: u32,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }

    pub fn into_patches(self) -> Vec<Patch> {
        self.patches
    }

    /// Counts patches whose `code` starts with `prefix`.
    pub fn count(&self, prefix: &str) -> usize {
        self.patches
            .iter()
            .filter(|p| p.code.starts_with(prefix))
            .count()
    }

    /// Records an edit made outside [`Editor::set`], e.g. a whole-file rewrite.
    pub fn record(&mut self, code: &'static str, path: &str, before: Node, after: Node, note: &str) {
        if self.quiet > 0 {
            return;
        }
        self.patches.push(Patch {
            code,
            path: path.to_owned(),
            before,
            after,
            note: note.to_owned(),
        });
    }

    /// Runs `f` with recording suppressed, for edits that are noise.
    pub fn quietly<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.quiet += 1;
        let out = f(self);
        self.quiet -= 1;
        out
    }

    /// Sets `root`'s node at `path`, recording the change. Returns whether the
    /// value actually changed.
    pub fn set(
        &mut self,
        root: &mut Value,
        code: &'static str,
        path: &str,
        value: Value,
        note: &str,
    ) -> bool {
        let before = get(root, path).cloned();
        let changed = match &before {
            None => true,
            Some(old) => *old != value,
        };
        if !changed {
            return false;
        }
        let before = match before {
            None => Node::Absent,
            Some(v) => Node::Present(v),
        };
        if !set(root, path, value.clone()) {
            return false;
        }
        self.record(code, path, before, Node::Present(value), note);
        true
    }

    /// Removes a node, recording it. Missing paths are a no-op.
    pub fn remove(&mut self, root: &mut Value, code: &'static str, path: &str, note: &str) -> bool {
        let Some(before) = get(root, path).cloned() else {
            return false;
        };
        if !remove(root, path) {
            return false;
        }
        self.record(code, path, Node::Present(before), Node::Absent, note);
        true
    }

    /// Renames an object key in place, recording it as one patch.
    pub fn rename(
        &mut self,
        root: &mut Value,
        code: &'static str,
        path: &str,
        to: &str,
        note: &str,
    ) -> bool {
        let Some(before) = get(root, path).cloned() else {
            return false;
        };
        let Some(object) = object_at(root, path) else {
            return false;
        };
        if !object.rename(last_segment(path), to) {
            return false;
        }
        self.record(code, path, Node::Present(before), Node::of(&Value::Null), note.trim_end().to_owned().as_str());
        // Report the rename as a code, not a value change: patch the note so a
        // reader sees both names.
        if let Some(p) = self.patches.last_mut() {
            p.note = format!("{note}: `{}` -> `{to}`", last_segment(path));
        }
        true
    }

    /// Runs a whole-value transform, recording one patch for the root when
    /// anything changed.
    pub fn transform(
        &mut self,
        root: &mut Value,
        code: &'static str,
        note: &str,
        mut f: impl FnMut(&mut Value) -> bool,
    ) -> bool {
        let before = root.clone();
        if !f(root) {
            return false;
        }
        self.record(code, "", Node::Present(before), Node::of(root), note);
        true
    }
}

/// Splits a path into unescaped segments.
pub fn split(path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    path.split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Joins segments, escaping `~` and `/`.
pub fn join(segments: &[&str]) -> String {
    segments
        .iter()
        .map(|s| s.replace('~', "~0").replace('/', "~1"))
        .collect::<Vec<_>>()
        .join("/")
}

/// Appends one segment to a path.
pub fn child(path: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    if path.is_empty() {
        escaped
    } else {
        format!("{path}/{escaped}")
    }
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Looks up a node. `None` when any segment is missing or the shape does not
/// match (an array index on an object, say).
pub fn get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut node = root;
    for segment in split(path) {
        node = match node {
            Value::Obj(o) => o.get(&segment)?,
            Value::Arr(a) => a.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(node)
}

fn object_at<'a>(root: &'a mut Value, path: &str) -> Option<&'a mut Object> {
    let parent_path = split(path);
    let (parent, _) = parent_path.split_at(parent_path.len().checked_sub(1)?);
    let mut node = root;
    for segment in parent {
        node = match node {
            Value::Obj(o) => o.get_mut(segment)?,
            Value::Arr(a) => a.get_mut(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    node.as_obj_mut()
}

/// Sets a node, creating missing intermediate objects. Array indices must
/// already exist (appending is a separate, explicit operation).
///
/// Returns `false` when a parent was missing or the final parent is not an
/// object/array.
pub fn set(root: &mut Value, path: &str, value: Value) -> bool {
    let segments = split(path);
    if segments.is_empty() {
        *root = value;
        return true;
    }
    let mut node = root;
    for (i, segment) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        match node {
            Value::Obj(o) => {
                if last {
                    o.insert(segment.clone(), value);
                    return true;
                }
                if !o.contains_key(segment) {
                    o.insert(segment.clone(), Value::Obj(Object::new()));
                }
                node = o.get_mut(segment).expect("just inserted");
            }
            Value::Arr(a) => {
                let Ok(index) = segment.parse::<usize>() else {
                    return false;
                };
                if index >= a.len() {
                    return false;
                }
                if last {
                    a[index] = value;
                    return true;
                }
                node = &mut a[index];
            }
            _ => return false,
        }
    }
    false
}

/// Removes a node. Returns whether it was there.
pub fn remove(root: &mut Value, path: &str) -> bool {
    let segments = split(path);
    let Some((last, parents)) = segments.split_last() else {
        return false;
    };
    let mut node = root;
    for segment in parents {
        node = match node {
            Value::Obj(o) => match o.get_mut(segment) {
                Some(n) => n,
                None => return false,
            },
            Value::Arr(a) => match segment.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                Some(n) => n,
                None => return false,
            },
            _ => return false,
        };
    }
    match node {
        Value::Obj(o) => o.remove(last).is_some(),
        Value::Arr(a) => match last.parse::<usize>() {
            Ok(i) if i < a.len() => {
                a.remove(i);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn escapes_and_splits() {
        let path = join(&["a/b", "c~d", "0"]);
        assert_eq!(path, "a~1b/c~0d/0");
        assert_eq!(split(&path), vec!["a/b", "c~d", "0"]);
    }

    #[test]
    fn gets_and_sets_through_mixed_shapes() {
        let mut v = parse(r#"{"a":[{"b":1}]}"#).unwrap();
        assert_eq!(get(&v, "a/0/b").unwrap().as_i64(), Some(1));
        assert!(set(&mut v, "a/0/b", Value::int(2)));
        assert_eq!(get(&v, "a/0/b").unwrap().as_i64(), Some(2));
        // Missing intermediates are created as objects.
        assert!(set(&mut v, "x/y/z", Value::int(3)));
        assert_eq!(get(&v, "x/y/z").unwrap().as_i64(), Some(3));
        // Out-of-range array indices are refused rather than extending.
        assert!(!set(&mut v, "a/5/b", Value::int(1)));
    }

    #[test]
    fn records_exactly_what_changed() {
        let mut v =
            parse(r##"{"type":"minecraft:crafting_shaped","key":{"#":{"item":"minecraft:x"}}}"##)
                .unwrap();
        let mut ed = Editor::new();
        assert!(ed.set(
            &mut v,
            "data.recipe.ingredient.object_to_string",
            "key/#",
            Value::str("minecraft:x"),
            "26.3 reads a bare id",
        ));
        // Setting the same value again records nothing.
        assert!(!ed.set(
            &mut v,
            "data.recipe.ingredient.object_to_string",
            "key/#",
            Value::str("minecraft:x"),
            "26.3 reads a bare id",
        ));
        assert_eq!(ed.patches().len(), 1);
        let p = &ed.patches()[0];
        assert_eq!(p.before.display(80), r#"{"item":"minecraft:x"}"#);
        assert_eq!(p.after.display(80), r#""minecraft:x""#);
    }

    #[test]
    fn removes_and_renames() {
        let mut v = parse(r#"{"a":1,"b":2}"#).unwrap();
        let mut ed = Editor::new();
        assert!(ed.rename(&mut v, "x.rename", "a", "c", "renamed"));
        assert_eq!(crate::to_string(&v), r#"{"c":1,"b":2}"#);
        assert!(ed.remove(&mut v, "x.remove", "b", "dropped"));
        assert_eq!(crate::to_string(&v), r#"{"c":1}"#);
        assert!(!ed.remove(&mut v, "x.remove", "zzz", "dropped"));
        assert_eq!(ed.patches().len(), 2);
        assert!(ed.patches()[0].note.contains("`a` -> `c`"));
    }

    #[test]
    fn quiet_mode_suppresses_recording() {
        let mut v = parse("{}").unwrap();
        let mut ed = Editor::new();
        ed.quietly(|e| e.set(&mut v, "x", "a", Value::int(1), "noise"));
        assert!(ed.patches().is_empty());
        ed.set(&mut v, "x", "b", Value::int(2), "kept");
        assert_eq!(ed.patches().len(), 1);
    }
}
