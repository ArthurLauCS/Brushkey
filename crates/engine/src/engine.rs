use crate::{
    dict::Dict,
    table::{Table, part},
    user::User,
};
use std::{collections::HashSet, ops::Range};

/// Fuzzy pinyin pairs; bit i of `Options::fuzzy` lets `FUZZY[i]` feed the second
/// candidate. The first six pair initials, the rest pair finals.
pub const FUZZY: [(&str, &str); 11] = [
    ("z", "zh"), ("c", "ch"), ("s", "sh"), ("n", "l"), ("f", "h"), ("r", "l"),
    ("an", "ang"), ("en", "eng"), ("in", "ing"), ("ian", "iang"), ("uan", "uang"),
];

/// Score penalties (natural log) per syllable typed as its initial only, per
/// syllable read through a fuzzy pair, per syllable typed with two adjacent
/// letters swapped, and for a last syllable still being typed.
const ABBR: f32 = -3.0;
const FUZZ: f32 = -1.0;
const SWAP: f32 = -3.0;
const PART: f32 = -1.0;

fn is_initial(s: &str) -> bool {
    matches!(s, "zh" | "ch" | "sh") || s.len() == 1 && "bpmfdtnlgkhjqxrzcsyw".contains(s)
}

/// `a` then `b` (pinyin letters) in `buf`, without allocating.
fn cat<'a>(buf: &'a mut [u8; 8], a: &[u8], b: &[u8]) -> &'a str {
    let n = a.len() + b.len();
    buf[..a.len()].copy_from_slice(a);
    buf[a.len()..n].copy_from_slice(b);
    std::str::from_utf8(&buf[..n]).unwrap()
}

/// Whole syllables have a vowel; m, n, ng, hm, hng (interjections) don't.
fn has_vowel(s: &str) -> bool {
    s.bytes().any(|b| b"aeiouv".contains(&b))
}

#[derive(Clone, Copy, Default)]
pub struct Options {
    /// Show candidates in Traditional characters with Taiwan forms and vocabulary.
    pub trad: bool,
    pub fuzzy: u32,
    /// A password field: picks are not remembered.
    pub private: bool,
}

pub struct Cand {
    pub text: String,
    simp: String,
    key: Vec<u16>,
    /// Pinyin byte offset where this candidate ends.
    end: usize,
}

/// A word of the composition: picked, kept as shown before a later pick, or
/// (in `Engine::rest`) what Space commits for the pinyin after them.
struct Seg {
    text: String,
    simp: String,
    /// Empty when the word cannot be learned.
    key: Vec<u16>,
    /// The pinyin bytes it reads.
    start: usize,
    end: usize,
    /// Picked by the user, so learned on its own.
    pick: bool,
}

/// One syllable of the lattice: the pinyin up to `end` reads as syllable ids lo..=hi.
#[derive(Clone, Copy)]
struct Syl {
    end: usize,
    lo: u16,
    hi: u16,
    pen: f32,
    /// Read through a fuzzy pair.
    fuzzy: bool,
    /// Two syllables with the letters at their boundary swapped (nhiao for ni
    /// hao): the second one; lo is the first.
    next: Option<u16>,
}

/// The first word of the best sentence from a lattice node: entry `e` of table
/// `ti` up to node `to`, its syllables, and the sentence's score.
#[derive(Clone)]
struct Step {
    score: f32,
    to: usize,
    ti: usize,
    e: usize,
    syls: Vec<Syl>,
    /// In the sentences that read a syllable through a fuzzy pair: one has been
    /// read, so the sentence goes on with the best exact one from `to`.
    exact: bool,
}

/// A word of a best sentence: entry `e` of table `ti`, its start node and syllables.
type Word = (usize, usize, usize, Vec<Syl>);

/// Depth-first walk of table `t` along the lattice from `node`, where entries
/// lo..hi share their first `d` syllables and are all longer. Calls
/// `f(end, a, b, pen, path)` for every key that ends on a lattice node; a..b are
/// that key's entries and `path` its syllables.
fn walk(
    t: &Table,
    syl: &[Vec<Syl>],
    node: usize,
    d: usize,
    lo: usize,
    hi: usize,
    pen: f32,
    path: &mut Vec<Syl>,
    f: &mut impl FnMut(usize, usize, usize, f32, &[Syl]),
) {
    for s in &syl[node] {
        path.push(*s);
        let mut a = part(lo, hi, |e| t.key_at(e, d) < s.lo);
        while a < hi {
            let v = t.key_at(a, d);
            if v > s.hi {
                break;
            }
            let b = part(a, hi, |e| t.key_at(e, d) <= v);
            // a1..b1: the entries that also have the second of a pair at d + 1
            let (a1, b1, n) = match s.next {
                None => (a, b, d + 1),
                Some(w) => {
                    let x = part(a, b, |e| t.key_len(e) == d + 1);
                    let a1 = part(x, b, |e| t.key_at(e, d + 1) < w);
                    (a1, part(a1, b, |e| t.key_at(e, d + 1) <= w), d + 2)
                }
            };
            let x = part(a1, b1, |e| t.key_len(e) == n);
            if x > a1 {
                f(s.end, a1, x, pen + s.pen, path);
            }
            if x < b1 && !syl[s.end].is_empty() {
                walk(t, syl, s.end, n, x, b1, pen + s.pen, path, f);
            }
            a = b;
        }
        path.pop();
    }
}

/// Best sentences from each lattice node to `end`, by dynamic programming from
/// the back, in one pass: `[0]` reads every syllable as typed, `[1]` at least
/// one through a fuzzy pair.
fn best(tables: &[&Table; 2], syl: &[Vec<Syl>], end: usize) -> [Vec<Option<Step>>; 2] {
    let mut best = [vec![None; end + 1], vec![None; end + 1]];
    best[0][end] = Some(Step { score: 0.0, to: end, ti: 0, e: 0, syls: vec![], exact: false });
    let score = |s: &Option<Step>| s.as_ref().map_or(f32::NEG_INFINITY, |s| s.score);
    for i in (0..end).rev() {
        for (ti, t) in tables.iter().enumerate() {
            walk(t, syl, i, 0, 0, t.len(), 0.0, &mut vec![], &mut |j, e, _, pen, path| {
                let (x, f) = (score(&best[0][j]), score(&best[1][j]));
                let mut put = |k: usize, rest: f32, exact: bool| {
                    let s = t.score(e) + pen + rest;
                    if s > score(&best[k][i]) {
                        best[k][i] = Some(Step { score: s, to: j, ti, e, syls: path.to_vec(), exact });
                    }
                };
                // A word read through a fuzzy pair goes on with either sentence; others keep theirs.
                if path.iter().any(|s| s.fuzzy) {
                    put(1, x.max(f), x >= f);
                } else {
                    put(0, x, false);
                    put(1, f, false);
                }
            });
        }
    }
    best
}

/// The words of the best sentence from node `i` (see `best`), one read through
/// a fuzzy pair if `fuzzy`.
fn path(best: &[Vec<Option<Step>>; 2], fuzzy: bool, mut i: usize) -> Vec<Word> {
    let (mut t, mut words) = (&best[fuzzy as usize], vec![]);
    while let Some(s) = &t[i]
        && s.to > i
    {
        words.push((s.ti, s.e, i, s.syls.clone()));
        if s.exact {
            t = &best[0];
        }
        i = s.to;
    }
    words
}

/// Pinyin composition: typed letters, words fixed so far, and candidates for the rest.
pub struct Engine {
    dict: &'static Dict,
    pub user: User,
    opts: Options,
    raw: String,
    segs: Vec<Seg>,
    /// What Space commits for the pinyin after `segs`.
    rest: Vec<Seg>,
    /// Adjusting the sentence (← →): the focused word, counting `segs`, then `rest`.
    focus: Option<usize>,
    cands: Vec<Cand>,
    pending: Pending,
    /// Syllable boundaries of the best sentence for the remaining pinyin, for the preedit.
    seps: Vec<usize>,
}

impl Engine {
    pub fn new(dict: &'static Dict, mut user: User, opts: Options) -> Engine {
        user.refresh(dict);
        let (raw, segs, rest, focus, cands, pending, seps) = Default::default();
        Engine { dict, user, opts, raw, segs, rest, focus, cands, pending, seps }
    }

    pub fn set_options(&mut self, opts: Options) {
        self.opts = opts;
        self.update();
    }

    pub fn options(&self) -> Options {
        self.opts
    }

    pub fn is_composing(&self) -> bool {
        !self.raw.is_empty()
    }

    /// Adds a typed letter (a-z, v for ü) or a syllable separator `'` at the end,
    /// back to typing.
    pub fn push(&mut self, c: char) {
        if self.raw.is_empty() {
            self.user.refresh(self.dict);
        }
        self.focus = None;
        self.raw.push(c);
        self.update();
    }

    /// Backspace: removes the last letter, or, with no pinyin left, undoes the
    /// last pick, with the words kept as shown before it; back to typing.
    pub fn pop(&mut self) {
        self.focus = None;
        if self.raw.len() > self.pos() {
            self.raw.pop();
        } else {
            self.segs.pop();
            while self.segs.last().is_some_and(|s| !s.pick) {
                self.segs.pop();
            }
        }
        self.update();
    }

    pub fn clear(&mut self) {
        self.raw.clear();
        self.segs.clear();
        self.focus = None;
        self.update();
    }

    pub fn adjusting(&self) -> bool {
        self.focus.is_some()
    }

    /// ←: starts adjusting at the last word, or focuses the previous one.
    pub fn left(&mut self) {
        let n = self.segs.len() + self.rest.len();
        self.focus = self.focus.map_or(n.checked_sub(1), |k| Some(k.saturating_sub(1)));
        self.update();
    }

    /// →: focuses the next word; past the last one, back to typing.
    pub fn right(&mut self) {
        self.focus = self.focus.map(|k| k + 1).filter(|&k| k < self.segs.len() + self.rest.len());
        self.update();
    }

    /// Esc while adjusting: back to typing, keeping the words picked.
    pub fn leave(&mut self) {
        self.focus = None;
        self.update();
    }

    /// Space or Enter while adjusting: commits the sentence as shown.
    pub fn commit_all(&mut self) -> String {
        self.leave();
        self.select(0).unwrap_or_else(|| self.commit_raw())
    }

    /// While adjusting, the byte range of the focused word in `preedit`.
    pub fn focus(&self) -> Option<Range<usize>> {
        let k = self.focus?;
        let a: usize = self.words().take(k).map(|w| w.text.len()).sum();
        Some(a..a + self.word(k).text.len())
    }

    fn words(&self) -> impl Iterator<Item = &Seg> {
        self.segs.iter().chain(&self.rest)
    }

    fn word(&self, k: usize) -> &Seg {
        self.words().nth(k).unwrap()
    }

    /// What the composition shows: while typing, the words fixed so far, then the
    /// rest of the pinyin split into syllables ("你好shi'jie"); while adjusting,
    /// the sentence.
    pub fn preedit(&self) -> String {
        let mut s: String = self.segs.iter().map(|x| &*x.text).collect();
        if self.focus.is_some() {
            let end = self.words().last().map_or(0, |w| w.end);
            return self.rest.iter().fold(s, |s, w| s + &w.text) + &self.raw[end..];
        }
        let rest = &self.raw[self.pos()..];
        let end = rest.trim_end_matches('\'').len();
        let mut last = 0;
        for &e in &self.seps {
            s += &rest[last..e];
            if e < end && !rest[e..].starts_with('\'') {
                s.push('\'');
            }
            last = e;
        }
        s + &rest[last..]
    }

    /// Enter: the picks plus the remaining pinyin as typed, without learning.
    pub fn commit_raw(&mut self) -> String {
        let s: String = self.segs.iter().map(|x| &*x.text).chain([&self.raw[self.pos()..]]).collect();
        self.clear();
        s
    }

    /// Picks candidate i. Returns the text to commit once all pinyin is consumed.
    pub fn select(&mut self, i: usize) -> Option<String> {
        self.pick(i, None)
    }

    /// Picks the first (`last` false) or last character of candidate i (以词定字).
    pub fn select_char(&mut self, i: usize, last: bool) -> Option<String> {
        self.pick(i, Some(last))
    }

    /// Removes candidate i from the user's memory; dictionary words keep their dictionary rank.
    pub fn forget(&mut self, i: usize) {
        self.candidates(i + 1);
        let Some(c) = self.cands.get(i) else { return };
        let lines = format!("-\t{}\t{}\n=\t{}\t\n", self.dict.key_str(&c.key), c.simp, c.simp);
        self.user.write(self.dict, &lines);
        self.update();
    }

    fn pos(&self) -> usize {
        self.segs.last().map_or(0, |s| s.end)
    }

    /// Picks candidate i for the pinyin after the words fixed so far or, while
    /// adjusting, in place of the focused word. Commits once the words fixed
    /// cover all of the pinyin.
    fn pick(&mut self, i: usize, char_of: Option<bool>) -> Option<String> {
        self.candidates(i + 1);
        let c = self.cands.get(i)?;
        let start = self.focus.map_or(self.pos(), |k| self.word(k).start);
        let mut seg = Seg { text: c.text.clone(), simp: c.simp.clone(), key: c.key.clone(), start, end: c.end, pick: true };
        if let Some(last) = char_of {
            let t: Vec<char> = seg.text.chars().collect();
            let s: Vec<char> = seg.simp.chars().collect();
            let k = if last { t.len() - 1 } else { 0 };
            seg.text = t[k].into();
            // Learn the character only when the word lines up one character per syllable.
            if s.len() == t.len() && seg.key.len() == t.len() {
                seg.simp = s[k].into();
                seg.key = vec![seg.key[k]];
            } else {
                seg.key.clear();
            }
        }
        match self.focus {
            None => self.segs.push(seg),
            Some(k) => {
                // The words before the focus stay as shown; the picks after it that
                // the new word doesn't cover stay, with the best words between them.
                let mut words: Vec<Seg> = self.segs.drain(..).chain(self.rest.drain(..)).collect();
                let later: Vec<Seg> = words.split_off(k + 1).into_iter().filter(|w| w.pick && w.start >= seg.end).collect();
                words.truncate(k);
                words.push(seg);
                for w in later {
                    let end = words[words.len() - 1].end;
                    if w.start > end {
                        words.extend(self.fill(end, w.start));
                    }
                    words.push(w);
                }
                self.segs = words;
                self.focus = Some(k + 1);
            }
        }
        if self.pos() < self.raw.trim_end_matches('\'').len() {
            self.update();
            return None;
        }
        // Learn every pick, and the words together as one new word.
        let segs = std::mem::take(&mut self.segs);
        let mut lines = String::new();
        let mut learn = |key: &[u16], simp: &str, text: &str| {
            lines += &format!("+\t{}\t{simp}\n", self.dict.key_str(key));
            if self.opts.trad {
                lines += &format!("=\t{simp}\t{text}\n");
            }
        };
        for s in segs.iter().filter(|s| s.pick && !s.key.is_empty()) {
            learn(&s.key, &s.simp, &s.text);
        }
        if segs.len() > 1 && segs.iter().all(|s| !s.key.is_empty()) {
            let key: Vec<u16> = segs.iter().flat_map(|s| s.key.iter().copied()).collect();
            let simp: String = segs.iter().map(|s| &*s.simp).collect();
            let text: String = segs.iter().map(|s| &*s.text).collect();
            learn(&key, &simp, &text);
        }
        if !self.opts.private {
            self.user.write(self.dict, &lines);
        }
        self.clear();
        Some(segs.iter().map(|s| &*s.text).collect())
    }

    /// Syllable lattice of `raw`: for each byte offset, the syllables starting there,
    /// with those read through the `fuzzy` pairs. Also returns where the pinyin
    /// ends, ignoring trailing separators.
    fn lattice(&self, raw: &str, fuzzy: u32) -> (Vec<Vec<Syl>>, usize) {
        let end = raw.trim_end_matches('\'').len();
        let mut out: Vec<Vec<Syl>> = vec![vec![]; raw.len() + 1];
        // whole[i]: the j up to i + 12 where raw[i..j] splits into whole syllables,
        // the last one possibly unfinished at the end (nia for nian). Swapped
        // letters are read only where the letters don't split so (gou stays gou,
        // not guo).
        let mut whole: Vec<Vec<usize>> = vec![vec![]; raw.len() + 1];
        let b = raw.as_bytes();
        for i in (0..raw.len()).rev() {
            if b[i] == b'\'' {
                out[i] = out[i + 1].clone();
                continue;
            }
            for j in i + 1..=(i + 6).min(raw.len()) {
                let s = match &raw[i..j] {
                    s if s.contains('\'') => break,
                    "lue" => "lve",
                    "nue" => "nve",
                    s => s,
                };
                let id = self.dict.syl(s);
                if has_vowel(s) && (id.is_some() || j == end && self.dict.prefix(s).is_some()) {
                    let more: Vec<usize> = whole[j].iter().copied().filter(|&k| k <= i + 12).collect();
                    whole[i].push(j);
                    whole[i].extend(more);
                }
                let mut add = |p: Option<(u16, u16)>, pen, fuzzy| {
                    if let Some((lo, hi)) = p {
                        out[i].push(Syl { end: j, lo, hi, pen, fuzzy, next: None })
                    }
                };
                if let Some(id) = id {
                    add(Some((id, id)), 0.0, false);
                    for f in self.fuzzy(s, fuzzy) {
                        add(Some((f, f)), FUZZ, true);
                    }
                } else {
                    if j == end && !is_initial(s) {
                        add(self.dict.prefix(s), PART, false);
                    }
                    if has_vowel(s) && !whole[i].contains(&j) {
                        for k in 0..s.len() - 1 {
                            let mut t = [0; 8];
                            t[..s.len()].copy_from_slice(s.as_bytes());
                            t.swap(k, k + 1);
                            add(self.syl(std::str::from_utf8(&t[..s.len()]).unwrap()).map(|id| (id, id)), SWAP, false);
                        }
                    }
                }
                if is_initial(s) {
                    add(self.dict.prefix(s), ABBR, false);
                    for (k, &(a, b)) in FUZZY[..6].iter().enumerate() {
                        if fuzzy >> k & 1 == 1 && (s == a || s == b) {
                            add(self.dict.prefix(if s == a { b } else { a }), ABBR + FUZZ, true);
                        }
                    }
                }
            }
            // Two syllables with the letters at their boundary swapped: b[k] starts
            // the second, b[k + 1] ends the first (nhiao for ni hao).
            for k in i..(i + 6).min(raw.len() - 1) {
                if b[k] == b'\'' || b[k + 1] == b'\'' {
                    break;
                }
                let mut t = [0; 8];
                let first = cat(&mut t, &b[i..k], &b[k + 1..k + 2]);
                let Some(x) = self.syl(first).filter(|_| has_vowel(first)) else { continue };
                for j in k + 2..=(k + 7).min(raw.len()) {
                    if b[j - 1] == b'\'' {
                        break;
                    }
                    let second = cat(&mut t, &b[k..k + 1], &b[k + 2..j]);
                    if let Some(y) = self.syl(second).filter(|_| has_vowel(second))
                        && !whole[i].contains(&j)
                    {
                        out[i].push(Syl { end: j, lo: x, hi: x, pen: SWAP, fuzzy: false, next: Some(y) });
                    }
                }
            }
        }
        (out, end)
    }

    /// The id of a whole syllable as typed (lue and nue spell lve and nve).
    fn syl(&self, s: &str) -> Option<u16> {
        self.dict.syl(match s {
            "lue" => "lve",
            "nue" => "nve",
            s => s,
        })
    }

    /// Other syllables `s` stands for under the fuzzy pairs `m`.
    fn fuzzy(&self, s: &str, m: u32) -> Vec<u16> {
        if m == 0 {
            return vec![];
        }
        let n = match s.get(..2) {
            Some("zh" | "ch" | "sh") => 2,
            _ if is_initial(&s[..1]) => 1,
            _ => 0,
        };
        let swap = |x: &str, pairs: std::ops::Range<usize>| {
            let mut v = vec![x.to_string()];
            for k in pairs.filter(|k| m >> k & 1 == 1) {
                match FUZZY[k] {
                    (a, b) if x == a => v.push(b.into()),
                    (a, b) if x == b => v.push(a.into()),
                    _ => {}
                }
            }
            v
        };
        let mut out = vec![];
        for i in swap(&s[..n], 0..6) {
            for f in swap(&s[n..], 6..11) {
                let t = i.clone() + &f;
                if let Some(id) = self.dict.syl(&t).filter(|_| t != s) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// Finds the candidates for the pinyin from the focused word or, while typing,
    /// after the words fixed so far: the word shown there (adjusting) or the
    /// user's picks for all of it (newest first) and the best sentence (typing),
    /// then every word the pinyin starts with, ranked by the best sentence it
    /// begins; the best reading through fuzzy pairs goes second. Also finds `rest`.
    fn update(&mut self) {
        self.cands.clear();
        self.seps.clear();
        self.rest.clear();
        self.pending = Pending::default();
        let pos = self.pos();
        if self.raw.len() <= pos {
            self.focus = None;
            return;
        }
        // The lattice starts at the focused word when it is fixed, else after the words fixed.
        let base = self.focus.filter(|&k| k < self.segs.len()).map_or(pos, |k| self.segs[k].start);
        let raw = self.raw[base..].to_string();
        let (syl, end) = self.lattice(&raw, self.opts.fuzzy);
        let fuzzy = |path: &[Syl]| path.iter().any(|s| s.fuzzy);
        self.user.rebuild();
        let user = self.user.table();
        let tables = [&user, &self.dict.words];
        let dp = best(&tables, &syl, end);

        // What Space commits after the words fixed: the user's newest pick for all
        // of it, else the best sentence.
        let p = pos - base;
        let words = path(&dp, false, p);
        self.seps = words.iter().flat_map(|w| w.3.iter().map(|s| s.end - p)).collect();
        let mut newest = None;
        walk(&user, &syl, p, 0, 0, user.len(), 0.0, &mut vec![], &mut |j, a, b, _, path| {
            if j == end && !fuzzy(path) {
                newest = (a..b).chain(newest).max_by_key(|&e| self.user.seq(e));
            }
        });
        self.rest = match newest {
            Some(e) => vec![Seg { text: self.text(user.text(e)), simp: user.text(e).into(), key: user.key(e), start: pos, end: base + end, pick: false }],
            None => words.iter().map(|w| self.seg(&tables, w, base)).collect(),
        };
        self.focus = self.focus.filter(|&k| k < self.segs.len() + self.rest.len());
        let from = self.focus.map_or(pos, |k| self.word(k).start) - base;

        // Words the pinyin starts with, ranked by their score plus the best
        // sentence for the rest: 我 (+是) lands near 我是, ahead of rare words
        // such as 握实. The user's picks for all of it go first, newest first.
        let mut found = vec![];
        for (ti, t) in tables.iter().enumerate() {
            walk(t, &syl, from, 0, 0, t.len(), 0.0, &mut vec![], &mut |j, a, b, pen, path| {
                if !fuzzy(path) {
                    let rest = dp[0][j].as_ref().map_or(f32::NEG_INFINITY, |b| b.score);
                    found.extend((a..b).map(|e| (base + j, ti, e, t.score(e) + pen + rest)));
                }
            });
        }
        let mine = |f: &(usize, usize, usize, f32)| f.1 == 0 && f.0 == base + end;
        found.sort_by(|x, y| {
            mine(y).cmp(&mine(x))
                .then_with(|| match mine(x) {
                    true => self.user.seq(y.2).cmp(&self.user.seq(x.2)),
                    false => y.3.total_cmp(&x.3).then(y.0.cmp(&x.0)),
                })
        });
        let (at, sentence) = match self.focus {
            Some(k) => {
                let w = self.word(k);
                (0, Some(Cand { text: w.text.clone(), simp: w.simp.clone(), key: w.key.clone(), end: w.end }))
            }
            None => (found.iter().take_while(|f| mine(f)).count(), (words.len() > 1).then(|| self.join(&tables, &raw, base, &words))),
        };
        let fuzzy = Some(path(&dp, true, from)).filter(|w| !w.is_empty()).map(|w| self.join(&tables, &raw, base, &w));
        self.pending = Pending { at, found: found.into_iter().map(|f| (f.0, f.1, f.2)).collect(), sentence, fuzzy, ..Default::default() };
    }

    /// The best words for the pinyin a..b, kept as shown.
    fn fill(&self, a: usize, b: usize) -> Vec<Seg> {
        let (syl, end) = self.lattice(&self.raw[a..b], 0);
        let user = self.user.table();
        let tables = [&user, &self.dict.words];
        path(&best(&tables, &syl, end), false, 0).iter().map(|w| self.seg(&tables, w, a)).collect()
    }

    /// Word `w` of a sentence on the pinyin from `base`, not picked.
    fn seg(&self, tables: &[&Table; 2], (ti, e, i, syls): &Word, base: usize) -> Seg {
        let (t, simp) = (tables[*ti], tables[*ti].text(*e));
        Seg { text: self.text(simp), simp: simp.into(), key: t.key(*e), start: base + i, end: base + syls[syls.len() - 1].end, pick: false }
    }

    /// A sentence on `raw` (the pinyin from `base`) as one candidate. Syllables
    /// read through a fuzzy pair are keyed as typed, so a pick comes first the
    /// next time it is typed that way.
    fn join(&self, tables: &[&Table; 2], raw: &str, base: usize, words: &[Word]) -> Cand {
        let mut c = Cand { text: String::new(), simp: String::new(), key: vec![], end: 0 };
        for (ti, e, i, syls) in words {
            let (t, simp) = (tables[*ti], tables[*ti].text(*e));
            c.text += &self.text(simp);
            c.simp += simp;
            let (key, mut a, mut k) = (t.key(*e), *i, 0);
            for s in syls {
                let n = 1 + s.next.is_some() as usize;
                match s.fuzzy {
                    true => c.key.push(self.syl(raw[a..s.end].trim_start_matches('\'')).unwrap_or(key[k])),
                    false => c.key.extend(&key[k..k + n]),
                }
                (a, k) = (s.end, k + n);
            }
            c.end = base + a;
        }
        c
    }

    /// A word as shown: in Traditional, the user's preferred form.
    fn text(&self, simp: &str) -> String {
        match self.opts.trad {
            true => self.dict.trad(simp, self.user.pref(simp)).swap_remove(0),
            false => simp.into(),
        }
    }

    /// The first `n` candidates, or all there are if fewer. Candidates are built
    /// (and converted to Traditional) only as far as asked for, a page at a time.
    pub fn candidates(&mut self, n: usize) -> &[Cand] {
        let user = self.user.table();
        let tables = [&user, &self.dict.words];
        let p = &mut self.pending;
        while self.cands.len() < n && p.k <= p.found.len() {
            let k = p.k;
            p.k += 1;
            let new: Vec<Cand> = if k == p.at {
                p.sentence.take().into_iter().collect()
            } else {
                let (j, ti, e) = p.found[k - (k > p.at) as usize];
                let (t, simp) = (tables[ti], tables[ti].text(e));
                let texts = match self.opts.trad {
                    true => self.dict.trad(simp, self.user.pref(simp)),
                    false => vec![simp.to_string()],
                };
                texts.into_iter().map(|text| Cand { text, simp: simp.into(), key: t.key(e), end: j }).collect()
            };
            for c in new {
                if p.seen.insert(c.text.clone()) {
                    self.cands.push(c);
                }
            }
            if !self.cands.is_empty()
                && let Some(c) = p.fuzzy.take()
                && p.seen.insert(c.text.clone())
            {
                self.cands.insert(1, c);
            }
        }
        &self.cands[..n.min(self.cands.len())]
    }
}

/// Candidates not built yet: words from the start of the pinyin as (end, table,
/// entry) in display order, with the sentence (adjusting: the word shown) going
/// before `found[at]` and the fuzzy reading second.
#[derive(Default)]
struct Pending {
    found: Vec<(usize, usize, usize)>,
    at: usize,
    sentence: Option<Cand>,
    fuzzy: Option<Cand>,
    k: usize,
    seen: HashSet<String>,
}
