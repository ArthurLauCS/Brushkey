use crate::table::{self, Table};
use std::{fs, io, path::Path};

const MAGIC: &[u8; 4] = b"PYD1";

/// Compiled dictionary, read in place from one byte buffer (the IME memory-maps
/// the file, so the pages are shared by every process). Sections, each prefixed
/// by its u32 length: syllables (sorted, '\n'-joined; index = syllable id), the
/// word table (key = syllable ids, score = ln of the word's corpus share), and
/// the OpenCC tables (key = UTF-16 of the source text, text = space-separated options).
pub struct Dict {
    syls: Vec<&'static str>,
    pub(crate) words: Table<'static>,
    /// Simplified → Traditional, phrases and characters.
    pub(crate) st: Table<'static>,
    /// Traditional → Taiwan vocabulary (軟件 → 軟體).
    pub(crate) tw: Table<'static>,
    /// Taiwan character forms (着 → 著).
    pub(crate) twv: Table<'static>,
}

impl Dict {
    /// Builds the dictionary file from the sources in `data/`.
    pub fn compile(dir: &Path) -> io::Result<Vec<u8>> {
        let mut src = vec![];
        for f in ["8105", "base"] {
            src.push(fs::read_to_string(dir.join(format!("rime-ice/{f}.dict.yaml")))?);
        }
        let mut words = vec![];
        for s in &src {
            // Entries follow the YAML header, which ends with a "..." line.
            for line in s.split_once("\n...").map_or("", |x| x.1).lines() {
                let mut c = line.split('\t');
                let (Some(text), Some(py)) = (c.next(), c.next()) else { continue };
                if text.is_empty() || text.starts_with('#') || py.is_empty() {
                    continue;
                }
                let w: f64 = c.next().and_then(|w| w.trim().parse().ok()).unwrap_or(0.0);
                words.push((text, py, w + 1.0));
            }
        }
        let mut syls: Vec<&str> = words.iter().flat_map(|w| w.1.split(' ')).collect();
        syls.sort_unstable();
        syls.dedup();
        let total: f64 = words.iter().map(|w| w.2).sum();
        let mut items: Vec<_> = words
            .iter()
            .map(|&(t, py, w)| {
                let key = py.split(' ').map(|s| syls.binary_search(&s).unwrap() as u16).collect();
                (key, t.to_string(), (w / total).ln() as f32)
            })
            .collect();
        items.sort_by(|a, b| b.2.total_cmp(&a.2)); // best first among equal keys
        let opencc = |files: &[&str]| -> io::Result<Vec<u8>> {
            let mut items = vec![];
            for f in files {
                for line in fs::read_to_string(dir.join("opencc").join(f))?.lines() {
                    if let Some((k, v)) = line.split_once('\t').filter(|_| !line.starts_with('#')) {
                        items.push((k.encode_utf16().collect(), v.to_string(), 0.0));
                    }
                }
            }
            Ok(table::build(items))
        };
        let sections = [
            syls.join("\n").into_bytes(),
            table::build(items),
            opencc(&["STPhrases.txt", "STCharacters.txt"])?,
            opencc(&["TWPhrases.txt"])?,
            opencc(&["TWVariants.txt"])?,
        ];
        let mut out = MAGIC.to_vec();
        for s in sections {
            out.extend((s.len() as u32).to_le_bytes());
            out.extend(s);
        }
        Ok(out)
    }

    /// Reads a buffer made by `compile`; None if it is not one.
    pub fn load(mut b: &'static [u8]) -> Option<Dict> {
        b = b.strip_prefix(MAGIC)?;
        let mut sec = || {
            let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
            let s = b.get(4..4 + n)?;
            b = &b[4 + n..];
            Some(s)
        };
        let syls = std::str::from_utf8(sec()?).ok()?.split('\n').collect();
        let [words, st, tw, twv] = [sec()?, sec()?, sec()?, sec()?].map(Table::new);
        Some(Dict { syls, words, st, tw, twv })
    }

    pub(crate) fn syl(&self, s: &str) -> Option<u16> {
        self.syls.binary_search(&s).ok().map(|i| i as u16)
    }

    pub(crate) fn syl_str(&self, id: u16) -> &'static str {
        self.syls[id as usize]
    }

    /// Ids of the syllables starting with `p`, inclusive (sorting keeps them together).
    pub(crate) fn prefix(&self, p: &str) -> Option<(u16, u16)> {
        let a = self.syls.partition_point(|s| *s < p);
        let b = self.syls.partition_point(|s| *s < p || s.starts_with(p));
        (a < b).then(|| (a as u16, b as u16 - 1))
    }

    /// Syllable ids of a space-separated pinyin key such as "ruan jian".
    pub(crate) fn parse_key(&self, k: &str) -> Option<Vec<u16>> {
        k.split(' ').map(|s| self.syl(s)).collect()
    }

    pub(crate) fn key_str(&self, key: &[u16]) -> String {
        key.iter().map(|&s| self.syl_str(s)).collect::<Vec<_>>().join(" ")
    }
}
