//! Named typed arrays per cell and layer, with grouped, sparse, undoable edits.
//!
//! Layout: `data[cell * layers + layer]`. All values are f64 in Part 1
//! (decision: tight conservation tests; revisit f32 for GPU in Part 3+).
//!
//! History model: every mutation happens inside an edit *group*; closing the
//! group yields at most one commit with a monotonically increasing id.
//! Mutating calls outside an explicit group auto-wrap in a one-call group.
use crate::error::{Error, Result};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub layers: usize,
    pub data: Vec<f64>,
    /// Id of the last commit that touched this field (0 = never committed).
    pub version: u64,
}

#[derive(Debug, Clone)]
enum Change {
    Create { name: String, layers: usize, data: Vec<f64> },
    Delta { name: String, entries: Vec<(u32, f64, f64)> }, // (index, old, new)
}

#[derive(Debug, Clone)]
struct Commit {
    id: u64,
    #[allow(dead_code)]
    label: String,
    changes: Vec<Change>,
}

enum Baseline { Existing(Vec<f64>), Created }

struct Group {
    label: String,
    baselines: BTreeMap<String, Baseline>,
}

/// Sparse difference between the current state and an older version.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    /// (flat index, old value, new value) for entries that differ.
    pub entries: Vec<(u32, f64, f64)>,
    /// The field did not exist at the older version (old values are 0.0).
    pub created: bool,
}

pub struct FieldStore {
    n_cells: usize,
    fields: BTreeMap<String, Field>,
    undo: VecDeque<Commit>,
    redo: Vec<Commit>,
    next_id: u64,
    base_id: u64,
    max_history: usize,
    group: Option<Group>,
}

impl FieldStore {
    pub fn new(n_cells: usize) -> Self {
        FieldStore {
            n_cells,
            fields: BTreeMap::new(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            next_id: 1,
            base_id: 0,
            max_history: 64,
            group: None,
        }
    }
    pub fn with_max_history(mut self, n: usize) -> Self { self.max_history = n.max(1); self }
    pub fn n_cells(&self) -> usize { self.n_cells }

    // ---- reads ----
    pub fn has(&self, name: &str) -> bool { self.fields.contains_key(name) }
    pub fn get(&self, name: &str) -> Option<&Field> { self.fields.get(name) }
    pub fn names(&self) -> Vec<String> { self.fields.keys().cloned().collect() }
    pub fn data(&self, name: &str) -> Result<&[f64]> {
        self.fields.get(name).map(|f| f.data.as_slice()).ok_or_else(|| Error::UnknownField(name.into()))
    }
    pub fn layers(&self, name: &str) -> Result<usize> {
        self.fields.get(name).map(|f| f.layers).ok_or_else(|| Error::UnknownField(name.into()))
    }
    /// Id of the newest commit in the undo stack (state "as of" this version).
    pub fn current_version(&self) -> u64 { self.undo.back().map(|c| c.id).unwrap_or(self.base_id) }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }

    // ---- groups ----
    pub fn begin_group(&mut self, label: &str) -> Result<()> {
        if self.group.is_some() {
            return Err(Error::GroupState("a group is already open".into()));
        }
        self.group = Some(Group { label: label.to_string(), baselines: BTreeMap::new() });
        Ok(())
    }

    /// Closes the open group; returns the commit id, or None if nothing changed.
    pub fn end_group(&mut self) -> Result<Option<u64>> {
        let g = self.group.take().ok_or_else(|| Error::GroupState("no group is open".into()))?;
        let mut changes: Vec<Change> = Vec::new();
        let mut touched: Vec<String> = Vec::new();
        for (name, base) in g.baselines.into_iter() {
            let cur = match self.fields.get(&name) {
                Some(f) => f,
                None => continue,
            };
            match base {
                Baseline::Existing(old) => {
                    let mut entries = Vec::new();
                    for (i, (o, nw)) in old.iter().zip(cur.data.iter()).enumerate() {
                        if o.to_bits() != nw.to_bits() {
                            entries.push((i as u32, *o, *nw));
                        }
                    }
                    if !entries.is_empty() {
                        touched.push(name.clone());
                        changes.push(Change::Delta { name, entries });
                    }
                }
                Baseline::Created => {
                    touched.push(name.clone());
                    changes.push(Change::Create { name, layers: cur.layers, data: cur.data.clone() });
                }
            }
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let id = self.next_id;
        self.next_id += 1;
        for name in touched {
            if let Some(f) = self.fields.get_mut(&name) {
                f.version = id;
            }
        }
        self.undo.push_back(Commit { id, label: g.label, changes });
        self.redo.clear();
        while self.undo.len() > self.max_history {
            if let Some(c) = self.undo.pop_front() {
                self.base_id = c.id;
            }
        }
        Ok(Some(id))
    }

    fn with_auto<T>(&mut self, label: &str, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let auto = self.group.is_none();
        if auto {
            self.begin_group(label)?;
        }
        let res = f(self);
        if auto {
            let end = self.end_group();
            if res.is_ok() {
                end?;
            }
        }
        res
    }

    fn touch(&mut self, name: &str) -> Result<()> {
        let field = self.fields.get(name).ok_or_else(|| Error::UnknownField(name.into()))?;
        if let Some(g) = self.group.as_mut() {
            if !g.baselines.contains_key(name) {
                g.baselines.insert(name.to_string(), Baseline::Existing(field.data.clone()));
            }
        }
        Ok(())
    }

    // ---- writes ----
    pub fn create(&mut self, name: &str, layers: usize, init: f64) -> Result<()> {
        let n = self.n_cells;
        self.with_auto("create", |s| s.create_inner(name, layers, vec![init; n * layers.max(1)]))
    }

    fn create_inner(&mut self, name: &str, layers: usize, data: Vec<f64>) -> Result<()> {
        if layers == 0 {
            return Err(Error::InvalidParam("layers must be >= 1".into()));
        }
        if self.fields.contains_key(name) {
            return Err(Error::InvalidParam(format!("field '{name}' already exists")));
        }
        if let Some(g) = self.group.as_mut() {
            g.baselines.insert(name.to_string(), Baseline::Created);
        }
        self.fields.insert(name.to_string(), Field { layers, data, version: 0 });
        Ok(())
    }

    /// Mutate a field in place through a closure.
    pub fn write<F: FnOnce(&mut [f64])>(&mut self, name: &str, f: F) -> Result<()> {
        self.with_auto("write", |s| {
            s.touch(name)?;
            let fld = s.fields.get_mut(name).ok_or_else(|| Error::UnknownField(name.into()))?;
            f(&mut fld.data);
            Ok(())
        })
    }

    /// Create the field if missing, otherwise replace its contents.
    pub fn put(&mut self, name: &str, layers: usize, data: Vec<f64>) -> Result<()> {
        if data.len() != self.n_cells * layers {
            return Err(Error::InvalidParam(format!("field '{name}': length {} != n_cells*layers", data.len())));
        }
        self.with_auto("put", |s| {
            if s.fields.contains_key(name) {
                if s.fields[name].layers != layers {
                    return Err(Error::InvalidParam(format!("field '{name}': layer count mismatch")));
                }
                s.touch(name)?;
                if let Some(f) = s.fields.get_mut(name) {
                    f.data = data;
                }
                Ok(())
            } else {
                s.create_inner(name, layers, data)
            }
        })
    }

    // ---- history ----
    fn recompute_version(&mut self, name: &str) {
        let mut v = 0;
        for c in self.undo.iter().rev() {
            let hit = c.changes.iter().any(|ch| match ch {
                Change::Create { name: n, .. } | Change::Delta { name: n, .. } => n == name,
            });
            if hit {
                v = c.id;
                break;
            }
        }
        if let Some(f) = self.fields.get_mut(name) {
            f.version = v;
        }
    }

    pub fn undo(&mut self) -> Result<Option<u64>> {
        if self.group.is_some() {
            return Err(Error::GroupState("cannot undo while a group is open".into()));
        }
        let commit = match self.undo.pop_back() {
            Some(c) => c,
            None => return Ok(None),
        };
        let mut names = Vec::new();
        for ch in commit.changes.iter().rev() {
            match ch {
                Change::Create { name, .. } => {
                    self.fields.remove(name);
                }
                Change::Delta { name, entries } => {
                    if let Some(f) = self.fields.get_mut(name) {
                        for &(i, old, _) in entries {
                            f.data[i as usize] = old;
                        }
                    }
                    names.push(name.clone());
                }
            }
        }
        for n in names {
            self.recompute_version(&n);
        }
        let id = commit.id;
        self.redo.push(commit);
        Ok(Some(id))
    }

    pub fn redo(&mut self) -> Result<Option<u64>> {
        if self.group.is_some() {
            return Err(Error::GroupState("cannot redo while a group is open".into()));
        }
        let commit = match self.redo.pop() {
            Some(c) => c,
            None => return Ok(None),
        };
        for ch in commit.changes.iter() {
            match ch {
                Change::Create { name, layers, data } => {
                    self.fields.insert(name.clone(), Field { layers: *layers, data: data.clone(), version: commit.id });
                }
                Change::Delta { name, entries } => {
                    if let Some(f) = self.fields.get_mut(name) {
                        for &(i, _, nw) in entries {
                            f.data[i as usize] = nw;
                        }
                        f.version = commit.id;
                    }
                }
            }
        }
        let id = commit.id;
        self.undo.push_back(commit);
        Ok(Some(id))
    }

    /// Differences between the current state and the state as of `since`
    /// (a value previously returned by `current_version()` or a commit id).
    pub fn diff(&self, name: &str, since: u64) -> Result<Diff> {
        let cur = self.fields.get(name).ok_or_else(|| Error::UnknownField(name.into()))?;
        if since < self.base_id {
            return Err(Error::HistoryTruncated { requested: since, oldest_available: self.base_id });
        }
        let mut oldest: BTreeMap<u32, f64> = BTreeMap::new();
        let mut created = false;
        for c in self.undo.iter().rev() {
            if c.id <= since {
                break;
            }
            for ch in c.changes.iter() {
                match ch {
                    Change::Delta { name: n, entries } if n.as_str() == name => {
                        for &(i, old, _) in entries {
                            oldest.insert(i, old); // walking backwards: last write = oldest
                        }
                    }
                    Change::Create { name: n, .. } if n.as_str() == name => created = true,
                    _ => {}
                }
            }
        }
        let mut entries = Vec::new();
        if created {
            // Field did not exist at `since`: report the whole field against 0.0.
            for (i, &v) in cur.data.iter().enumerate() {
                if v != 0.0 {
                    entries.push((i as u32, 0.0, v));
                }
            }
        } else {
            for (&i, &old) in oldest.iter() {
                let nw = cur.data[i as usize];
                if old.to_bits() != nw.to_bits() {
                    entries.push((i, old, nw));
                }
            }
        }
        Ok(Diff { entries, created })
    }
}
