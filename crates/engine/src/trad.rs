use crate::{dict::Dict, table::Table};
use std::collections::HashSet;

/// Forward maximum matching of `s` against `t`, taking the k-th option (or the
/// last there is) of every match. Returns the result and the most options any
/// match had (0 = nothing matched).
fn mm(t: &Table, s: &str, k: usize) -> (String, usize) {
    let u: Vec<u16> = s.encode_utf16().collect();
    let (mut out, mut most, mut i) = (String::new(), 0, 0);
    while i < u.len() {
        if let Some((l, e)) = t.longest(&u[i..]) {
            let o: Vec<&str> = t.text(e).split(' ').collect();
            most = most.max(o.len());
            out += o[k.min(o.len() - 1)];
            i += l;
        } else {
            let c = char::decode_utf16(u[i..].iter().copied()).next().unwrap().unwrap_or('\u{FFFD}');
            out.push(c);
            i += c.len_utf16();
        }
    }
    (out, most)
}

impl Dict {
    /// Traditional (Taiwan) forms of a Simplified word, best first: the user's
    /// earlier pick `pref`, then Taiwan vocabulary (软件 → 軟體), then the
    /// character-for-character form (軟件). A single character also lists every
    /// Traditional character it stands for (干 → 幹 乾 干 榦).
    pub(crate) fn trad(&self, simp: &str, pref: Option<&str>) -> Vec<String> {
        let base = mm(&self.st, simp, 0).0;
        let mut out: Vec<String> = (0..mm(&self.tw, &base, 0).1).map(|k| mm(&self.tw, &base, k).0).collect();
        out.push(base);
        let u: Vec<u16> = simp.encode_utf16().collect();
        if simp.chars().count() == 1 && let Some(e) = self.st.find(&u) {
            out.extend(self.st.text(e).split(' ').map(String::from));
        }
        let mut seen = HashSet::new();
        pref.map(String::from)
            .into_iter()
            .chain(out.iter().map(|s| mm(&self.twv, s, 0).0))
            .filter(|s| seen.insert(s.clone()))
            .collect()
    }
}
