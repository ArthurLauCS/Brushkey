//! Sorted key → text table in a flat little-endian byte layout, read in place
//! (memory-mapped for the system dictionary, so every process shares one copy).
//!
//! Layout: entry count, key-pool length in u16 units, then one 16-byte record per
//! entry `[key_off u32, text_off u32, score f32, key_len u8, 0u8, text_len u16]`,
//! then the u16 key pool and the UTF-8 text pool. Entries are sorted by key
//! (lexicographic over u16, so a key sorts before its extensions); entries with
//! equal keys keep the order they were given to `build`.

const REC: usize = 16;

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

/// First index in lo..hi where `f` turns false (`f` must be true then false).
pub(crate) fn part(mut lo: usize, mut hi: usize, f: impl Fn(usize) -> bool) -> usize {
    while lo < hi {
        let m = lo + (hi - lo) / 2;
        if f(m) { lo = m + 1 } else { hi = m }
    }
    lo
}

pub(crate) struct Table<'a> {
    n: usize,
    recs: &'a [u8],
    keys: &'a [u8],
    texts: &'a [u8],
}

impl<'a> Table<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        let n = u32_at(b, 0) as usize;
        let k = 8 + n * REC;
        let t = k + u32_at(b, 4) as usize * 2;
        Table { n, recs: &b[8..k], keys: &b[k..t], texts: &b[t..] }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    fn rec(&self, e: usize) -> &'a [u8] {
        &self.recs[e * REC..e * REC + REC]
    }

    pub fn key_len(&self, e: usize) -> usize {
        self.rec(e)[12] as usize
    }

    pub fn key_at(&self, e: usize, d: usize) -> u16 {
        let i = (u32_at(self.rec(e), 0) as usize + d) * 2;
        u16::from_le_bytes([self.keys[i], self.keys[i + 1]])
    }

    pub fn key(&self, e: usize) -> Vec<u16> {
        (0..self.key_len(e)).map(|d| self.key_at(e, d)).collect()
    }

    pub fn score(&self, e: usize) -> f32 {
        f32::from_le_bytes(self.rec(e)[8..12].try_into().unwrap())
    }

    pub fn text(&self, e: usize) -> &'a str {
        let r = self.rec(e);
        let off = u32_at(r, 4) as usize;
        let len = u16::from_le_bytes([r[14], r[15]]) as usize;
        std::str::from_utf8(&self.texts[off..off + len]).unwrap_or("")
    }

    /// Longest key that is a prefix of `s`: (its length, its first entry).
    pub fn longest(&self, s: &[u16]) -> Option<(usize, usize)> {
        let (mut lo, mut hi, mut best) = (0, self.n, None);
        for (d, &v) in s.iter().enumerate() {
            lo = part(lo, hi, |e| self.key_len(e) == d);
            lo = part(lo, hi, |e| self.key_at(e, d) < v);
            hi = part(lo, hi, |e| self.key_at(e, d) <= v);
            if lo == hi {
                break;
            }
            if self.key_len(lo) == d + 1 {
                best = Some((d + 1, lo));
            }
        }
        best
    }

    /// First entry with exactly this key.
    pub fn find(&self, key: &[u16]) -> Option<usize> {
        self.longest(key).filter(|&(l, _)| l == key.len()).map(|(_, e)| e)
    }
}

/// Serializes `(key, text, score)` items; keys longer than 255 or texts longer
/// than 65535 bytes must not be passed.
pub(crate) fn build(mut items: Vec<(Vec<u16>, String, f32)>) -> Vec<u8> {
    items.sort_by(|a, b| a.0.cmp(&b.0)); // stable: equal keys keep the caller's order
    let (mut recs, mut keys, mut texts) = (Vec::<u8>::new(), Vec::<u16>::new(), Vec::<u8>::new());
    let mut koff = 0;
    for (i, (k, t, s)) in items.iter().enumerate() {
        if i == 0 || items[i - 1].0 != *k {
            koff = keys.len();
            keys.extend(k);
        }
        recs.extend((koff as u32).to_le_bytes());
        recs.extend((texts.len() as u32).to_le_bytes());
        recs.extend(s.to_le_bytes());
        recs.extend([k.len() as u8, 0]);
        recs.extend((t.len() as u16).to_le_bytes());
        texts.extend(t.as_bytes());
    }
    let mut out = (items.len() as u32).to_le_bytes().to_vec();
    out.extend((keys.len() as u32).to_le_bytes());
    out.extend(recs);
    out.extend(keys.iter().flat_map(|k| k.to_le_bytes()));
    out.extend(texts);
    out
}
