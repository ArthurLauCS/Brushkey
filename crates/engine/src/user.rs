use crate::{
    dict::Dict,
    table::{self, Table},
};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

/// Added to a learned word's score in sentence conversion: doubles its share.
const BONUS: f32 = std::f32::consts::LN_2;

/// The user's memory, kept in an append-only log shared by every process the IME
/// runs in. Lines are tab-separated: `+ key word` (picked; later lines are more
/// recent), `- key word` (forgotten), `= simplified traditional` (preferred
/// Traditional form; empty = none). Each process replays new lines before a
/// composition starts, so a pick in one app shows up in all of them. Every pick
/// appends, so once repeats dominate, the log is rewritten with one line per
/// word: every process replays it whole when the IME activates.
pub struct User {
    path: Option<PathBuf>,
    writable: bool,
    /// Log bytes replayed so far, and the last of them, to notice the log being replaced.
    off: u64,
    tail: Vec<u8>,
    seq: u64,
    /// (key, word) → (recency, score used by sentence conversion).
    words: HashMap<(Vec<u16>, String), (u64, f32)>,
    prefs: HashMap<String, String>,
    bytes: Vec<u8>,
    /// Recency of each table entry, in table order.
    seqs: Vec<u64>,
    dirty: bool,
}

impl User {
    /// `path` None keeps memory for this session only; `writable` false (sandboxed
    /// apps) reads the log but keeps new picks in this process.
    pub fn open(path: Option<PathBuf>, writable: bool) -> User {
        User {
            path,
            writable,
            off: 0,
            tail: vec![],
            seq: 0,
            words: HashMap::new(),
            prefs: HashMap::new(),
            bytes: table::build(vec![]),
            seqs: vec![],
            dirty: false,
        }
    }

    /// Replays log lines other processes appended since the last call.
    pub fn refresh(&mut self, d: &Dict) {
        let Some(p) = self.path.clone() else { return };
        let mut buf = vec![];
        match File::open(&p) {
            Ok(mut f) => {
                let from = self.off - self.tail.len() as u64;
                if f.seek(SeekFrom::Start(from)).and_then(|_| f.read_to_end(&mut buf)).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(_) => return,
        }
        if !buf.starts_with(&self.tail) {
            // Memory was cleared or the file replaced: start over.
            *self = User::open(Some(p), self.writable);
            return self.refresh(d);
        }
        let new = &buf[self.tail.len()..];
        let n = new.iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1); // whole lines only
        for line in String::from_utf8_lossy(&new[..n]).lines() {
            self.apply(d, line);
        }
        self.off += n as u64;
        let mut t = std::mem::take(&mut self.tail);
        t.extend_from_slice(&new[..n]);
        self.tail = t[t.len().saturating_sub(64)..].to_vec();
    }

    fn apply(&mut self, d: &Dict, line: &str) {
        let mut p = line.splitn(3, '\t');
        match (p.next(), p.next(), p.next()) {
            (Some("+"), Some(k), Some(w)) => {
                let Some(key) = d.parse_key(k) else { return };
                self.seq += 1;
                let seq = self.seq;
                self.words.entry((key, w.into())).and_modify(|v| v.0 = seq).or_insert_with_key(|(k, w)| (seq, score(d, k, w)));
            }
            (Some("-"), Some(k), Some(w)) => {
                let Some(key) = d.parse_key(k) else { return };
                self.words.remove(&(key, w.to_string()));
            }
            (Some("="), Some(s), Some("")) => {
                self.prefs.remove(s);
            }
            (Some("="), Some(s), Some(t)) => {
                self.prefs.insert(s.into(), t.into());
            }
            _ => return,
        }
        self.dirty = true;
    }

    /// Appends log lines and replays them; kept in this process only when the log can't be written.
    pub(crate) fn write(&mut self, d: &Dict, lines: &str) {
        let saved = self.writable
            && self.path.as_ref().is_some_and(|p| {
                p.parent().is_none_or(|dir| fs::create_dir_all(dir).is_ok())
                    && OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(p)
                        .and_then(|mut f| f.write_all(lines.as_bytes()))
                        .is_ok()
            });
        if saved {
            self.refresh(d);
            self.compact(d);
        } else {
            lines.lines().for_each(|l| self.apply(d, l));
        }
    }

    /// Rewrites the log as one line per word (oldest first) and preference once
    /// it holds far more picks than words. Other processes see the shorter file
    /// and replay it from the start. A pick another process appends during the
    /// rewrite is lost.
    fn compact(&mut self, d: &Dict) {
        let Some(p) = self.path.clone() else { return };
        if self.seq < 2 * self.words.len() as u64 + 10_000 {
            return;
        }
        let mut v: Vec<_> = self.words.iter().collect();
        v.sort_by_key(|&(_, &(seq, _))| seq);
        let mut s: String = v.iter().map(|((k, w), _)| format!("+\t{}\t{w}\n", d.key_str(k))).collect();
        s.extend(self.prefs.iter().map(|(a, b)| format!("=\t{a}\t{b}\n")));
        let tmp = p.with_extension(format!("{}.tmp", std::process::id()));
        if fs::write(&tmp, s).and_then(|_| fs::rename(&tmp, &p)).is_ok() {
            *self = User::open(Some(p), self.writable);
            self.refresh(d);
        } else {
            let _ = fs::remove_file(&tmp);
        }
    }

    pub(crate) fn rebuild(&mut self) {
        if !self.dirty {
            return;
        }
        let mut v: Vec<_> = self.words.iter().filter(|((k, w), _)| k.len() < 256 && w.len() < 65536).collect();
        v.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(b.1.0.cmp(&a.1.0))); // table order, newest first among equal keys
        self.seqs = v.iter().map(|&(_, &(seq, _))| seq).collect();
        self.bytes = table::build(v.into_iter().map(|((k, w), &(_, s))| (k.clone(), w.clone(), s)).collect());
        self.dirty = false;
    }

    /// The learned words as a table; call `rebuild` first.
    pub(crate) fn table(&self) -> Table<'_> {
        Table::new(&self.bytes)
    }

    /// How recently table entry `e` was picked (higher = newer).
    pub(crate) fn seq(&self, e: usize) -> u64 {
        self.seqs[e]
    }

    pub(crate) fn pref(&self, simp: &str) -> Option<&str> {
        self.prefs.get(simp).map(|s| s.as_str())
    }
}

/// A learned word scores as in the dictionary plus `BONUS`, so one pick of a rare
/// homophone (可疑) leaves sentences alone (可以帮我吗). A word the dictionary
/// lacks (picks joined into one) scores as the dictionary's best for its pinyin,
/// or as its pinyin typed character by character when that is higher, plus `BONUS`.
fn score(d: &Dict, key: &[u16], word: &str) -> f32 {
    let t = &d.words;
    let same_key = |e: &usize| t.key_len(*e) == key.len() && key.iter().enumerate().all(|(i, &s)| t.key_at(*e, i) == s);
    if let Some(e) = t.find(key).and_then(|e| (e..t.len()).take_while(same_key).find(|&e| t.text(e) == word)) {
        return t.score(e) + BONUS;
    }
    let top = |k: &[u16]| t.find(k).map(|e| t.score(e));
    let chars: f32 = key.iter().map(|&s| top(&[s]).unwrap_or_default()).sum();
    top(key).map_or(chars, |s| s.max(chars)) + BONUS
}
