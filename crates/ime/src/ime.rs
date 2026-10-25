//! Keystrokes to engine calls and output text. Knows nothing of TSF, so it is
//! tested on its own; `tip` feeds it keys and applies what comes out.

use crate::settings::{Settings, Toggle};
use engine::{Cand, Dict, Engine, Options, User};

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;

/// Keys that map to Chinese punctuation in Chinese mode.
const PUNCT: &str = ",.;:?!\\()<>[]^_$~`\"'";

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mode {
    pub chinese: bool,
    pub full: bool,
    pub trad: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Key {
    pub vk: u16,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub caps: bool,
}

/// What an eaten key does.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    /// A pinyin letter or the separator `'`.
    Push(char),
    Pop,
    Clear,
    /// Enter: commit the letters as typed.
    Raw,
    /// Pick candidate i of the page; Space is Pick(0).
    Pick(usize),
    /// `[` `]`: first / last character of the page's first candidate.
    Char(bool),
    Forget(usize),
    /// Next page when true.
    Page(bool),
    /// ← → adjust the sentence word by word; → when true.
    Move(bool),
    /// Esc while adjusting: back to typing.
    Leave,
    /// Space or Enter while adjusting: commit the sentence as shown.
    All,
    /// Commit the first candidate (adjusting: the sentence), then this character.
    Then(char),
    /// Not composing: output this character (Chinese punctuation or full width).
    Emit(char),
    Chinese,
    Full,
    Trad,
    /// Eaten so it doesn't move the caret under the composition; does nothing.
    Nothing,
}

#[derive(Default, Debug)]
pub struct Out {
    pub commit: String,
    /// Mode hint to show near the caret.
    pub hint: Option<&'static str>,
}

pub struct Ime {
    pub engine: Engine,
    pub mode: Mode,
    page: usize,
    /// Candidates per page.
    pub per_page: usize,
    toggle: Toggle,
    /// The last key output a digit, so `.` `,` stay half-width (3.14).
    digit: bool,
    /// Whether the next `"` / `'` closes the quote.
    quotes: [bool; 2],
    /// The toggle key is down and no other key came since.
    alone: bool,
    /// Last key tested and whether it was eaten, so the key-down that follows
    /// decides the same even though the test changed `digit`.
    tested: Option<(u16, bool)>,
}

impl Ime {
    pub fn new(dict: &'static Dict, user: User, mode: Mode, s: &Settings) -> Ime {
        let engine = Engine::new(dict, user, Options { trad: mode.trad, fuzzy: s.fuzzy, ..Default::default() });
        Ime { engine, mode, page: 0, per_page: s.page, toggle: s.toggle, digit: false, quotes: [false; 2], alone: false, tested: None }
    }

    /// Settings changed in the settings app.
    pub fn configure(&mut self, s: &Settings) {
        self.engine.set_options(Options { fuzzy: s.fuzzy, ..self.engine.options() });
        (self.per_page, self.toggle, self.page) = (s.page, s.toggle, 0);
    }

    /// Mode changed from outside (another app, the language bar).
    pub fn set_mode(&mut self, mode: Mode) {
        if mode.trad != self.mode.trad {
            self.engine.set_options(Options { trad: mode.trad, ..self.engine.options() });
        }
        self.mode = mode;
    }

    /// 中/英 from the language bar.
    pub fn toggle(&mut self) -> Out {
        self.apply(Act::Chinese)
    }

    pub fn page_index(&self) -> usize {
        self.page
    }

    /// Candidates up to the end of the current page.
    pub fn cands(&mut self) -> &[Cand] {
        self.engine.candidates((self.page + 1) * self.per_page)
    }

    /// Whether a key-down will be eaten (OnTestKeyDown). Keys that pass through
    /// are noted here, since TSF doesn't call OnKeyDown for them.
    pub fn test(&mut self, k: Key) -> bool {
        self.alone = self.is_toggle(k.vk);
        let eat = self.act(k).is_some();
        if !eat {
            self.saw(k);
        }
        self.tested = Some((k.vk, eat));
        eat
    }

    /// Handles a key-down (OnKeyDown); None if it passes through to the app.
    pub fn down(&mut self, k: Key) -> Option<Out> {
        if self.tested.take() == Some((k.vk, false)) {
            return None;
        }
        self.alone = self.is_toggle(k.vk);
        match self.act(k) {
            Some(a) => Some(self.apply(a)),
            None => {
                self.saw(k);
                None
            }
        }
    }

    /// Handles a key-up; the toggle key (Shift or Ctrl) released with no other key
    /// toggles 中/英. The key-up itself always passes through, so the app doesn't
    /// see the key stuck down.
    pub fn up(&mut self, vk: u16) -> Option<Out> {
        let toggled = self.is_toggle(vk) && self.alone;
        self.alone = false;
        toggled.then(|| self.apply(Act::Chinese))
    }

    fn is_toggle(&self, vk: u16) -> bool {
        match self.toggle {
            Toggle::Shift => matches!(vk, 0x10 | 0xA0 | 0xA1),
            Toggle::Ctrl => matches!(vk, 0x11 | 0xA2 | 0xA3),
            Toggle::Off => false,
        }
    }

    fn saw(&mut self, k: Key) {
        if !matches!(k.vk, 0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5) {
            self.digit = matches!(ch(k.vk, k.shift), Some('0'..='9'));
        }
    }

    fn act(&self, k: Key) -> Option<Act> {
        use Act::*;
        let comp = self.engine.is_composing();
        if k.ctrl && k.shift && !k.alt {
            return match k.vk {
                0x46 => Some(Trad), // F
                0x31..=0x39 if comp => Some(Forget((k.vk - 0x31) as usize)),
                _ => None,
            };
        }
        if k.ctrl || k.alt {
            return None;
        }
        if k.shift && k.vk == VK_SPACE {
            return Some(Full);
        }
        let letter = (0x41..=0x5A).contains(&k.vk);
        let c = ch(k.vk, if letter { k.shift != k.caps } else { k.shift });
        let pinyin = self.mode.chinese && letter && !k.shift && !k.caps;
        if comp {
            let adjusting = self.engine.adjusting();
            return Some(match (k.vk, c) {
                (VK_BACK, _) => Pop,
                (VK_ESCAPE, _) if adjusting => Leave,
                (VK_ESCAPE, _) => Clear,
                (VK_RETURN | VK_SPACE, _) if adjusting => All,
                (VK_RETURN, _) => Raw,
                (VK_SPACE, _) => Pick(0),
                (VK_LEFT, _) => Move(false),
                (VK_RIGHT, _) => Move(true),
                (VK_PRIOR, _) | (_, Some('-' | ',')) => Page(false),
                (VK_NEXT, _) | (_, Some('=' | '.')) => Page(true),
                (_, Some('[')) => Char(false),
                (_, Some(']')) => Char(true),
                (_, Some(d @ '1'..='9')) => Pick(d as usize - '1' as usize),
                (_, Some('0')) => Nothing,
                (_, Some(c)) if pinyin || c == '\'' => Push(c),
                (_, Some(c)) => Then(c),
                // Tab, End, Home, ↑ ↓, Insert, Delete
                (VK_TAB | 0x23 | 0x24 | 0x26 | 0x28 | 0x2D | 0x2E, None) => Nothing,
                _ => return None,
            });
        }
        let c = c?;
        if pinyin {
            return Some(Push(c));
        }
        let punct = self.mode.chinese && PUNCT.contains(c) && !(self.digit && matches!(c, '.' | ','));
        (punct || self.mode.full).then_some(Emit(c))
    }

    fn apply(&mut self, a: Act) -> Out {
        use Act::*;
        let mut out = Out::default();
        let (at, per) = (self.page * self.per_page, self.per_page);
        let e = &mut self.engine;
        let has = |e: &mut Engine, i: usize| e.candidates(i + 1).len() > i;
        match a {
            Push(c) => e.push(c),
            Pop => e.pop(),
            Clear => e.clear(),
            Raw => out.commit = e.commit_raw(),
            // a number past the page's last candidate picks nothing
            Pick(i) if i < per && has(e, at + i) => out.commit = e.select(at + i).unwrap_or_default(),
            Pick(0) => out.commit = e.commit_raw(),
            Char(last) if has(e, at) => out.commit = e.select_char(at, last).unwrap_or_default(),
            Forget(i) if i < per && has(e, at + i) => e.forget(at + i),
            Page(true) if has(e, at + per) => self.page += 1,
            Page(false) => self.page = self.page.saturating_sub(1),
            Move(false) => e.left(),
            Move(true) => e.right(),
            Leave => e.leave(),
            All => out.commit = e.commit_all(),
            Then(c) => {
                let first = if e.adjusting() { Some(e.commit_all()) } else if has(e, at) { e.select(at) } else { None };
                out.commit = first.unwrap_or_else(|| e.commit_raw());
                out.commit += &self.text(c);
            }
            Emit(c) => out.commit = self.text(c),
            Chinese => {
                out.commit = e.commit_raw();
                self.mode.chinese ^= true;
                out.hint = Some(if self.mode.chinese { "中" } else { "英" });
            }
            Full => {
                self.mode.full ^= true;
                out.hint = Some(if self.mode.full { "全" } else { "半" });
            }
            Trad => {
                self.mode.trad ^= true;
                e.set_options(Options { trad: self.mode.trad, ..e.options() });
                out.hint = Some(if self.mode.trad { "繁" } else { "简" });
            }
            Pick(_) | Char(_) | Forget(_) | Page(true) | Nothing => {}
        }
        if !matches!(a, Page(_)) {
            self.page = 0;
        }
        if !out.commit.is_empty() {
            self.digit = out.commit.ends_with(|c: char| c.is_ascii_digit() || ('０'..='９').contains(&c));
        }
        out
    }

    /// The text a typed character outputs outside a composition.
    fn text(&mut self, c: char) -> String {
        if self.mode.chinese && !(self.digit && matches!(c, '.' | ',')) {
            if let Some(s) = self.punct(c) {
                return s.into();
            }
        }
        match c {
            ' ' if self.mode.full => '\u{3000}',
            '!'..='~' if self.mode.full => char::from_u32(c as u32 + 0xFEE0).unwrap(),
            _ => c,
        }
        .into()
    }

    fn punct(&mut self, c: char) -> Option<&'static str> {
        const QUOTES: [[[&str; 2]; 2]; 2] = [[["“", "”"], ["‘", "’"]], [["「", "」"], ["『", "』"]]];
        Some(match c {
            ',' => "，",
            '.' => "。",
            ';' => "；",
            ':' => "：",
            '?' => "？",
            '!' => "！",
            '\\' => "、",
            '(' => "（",
            ')' => "）",
            '<' => "《",
            '>' => "》",
            '[' => "【",
            ']' => "】",
            '^' => "……",
            '_' => "——",
            '$' => "￥",
            '~' => "～",
            '`' => "·",
            '"' | '\'' => {
                let i = (c == '\'') as usize;
                let close = self.quotes[i];
                self.quotes[i] ^= true;
                QUOTES[self.mode.trad as usize][i][close as usize]
            }
            _ => return None,
        })
    }
}

/// The character a key types on the US layout (the one Chinese (Simplified) uses).
fn ch(vk: u16, shift: bool) -> Option<char> {
    let pick = |a: &str, b: &str, i: usize| if shift { b } else { a }.chars().nth(i);
    match vk {
        0x20 => Some(' '),
        0x30..=0x39 => pick("0123456789", ")!@#$%^&*(", (vk - 0x30) as usize),
        0x41..=0x5A => {
            let c = (b'a' + (vk - 0x41) as u8) as char;
            Some(if shift { c.to_ascii_uppercase() } else { c })
        }
        0x60..=0x69 => char::from_digit((vk - 0x60) as u32, 10),
        0x6A => Some('*'),
        0x6B => Some('+'),
        0x6D => Some('-'),
        0x6E => Some('.'),
        0x6F => Some('/'),
        0xBA..=0xC0 => pick(";=,-./`", ":+<_>?~", (vk - 0xBA) as usize),
        0xDB..=0xDE => pick("[\\]'", "{|}\"", (vk - 0xDB) as usize),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn ime(chinese: bool) -> Ime {
        static D: OnceLock<&'static Dict> = OnceLock::new();
        let d = D.get_or_init(|| {
            let b = Dict::compile(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data").as_ref()).unwrap();
            Box::leak(Box::new(Dict::load(Box::leak(b.into_boxed_slice())).unwrap()))
        });
        Ime::new(d, User::open(None, true), Mode { chinese, full: false, trad: false }, &Settings::default())
    }

    fn key(c: char) -> Key {
        let (vk, shift) = match c {
            'a'..='z' => (c as u16 - 32, false),
            'A'..='Z' => (c as u16, true),
            ' ' => (VK_SPACE, false),
            '\n' => (VK_RETURN, false),
            _ => (0..=0xFF).find_map(|vk| [false, true].into_iter().find(|&s| ch(vk, s) == Some(c)).map(|s| (vk, s))).unwrap(),
        };
        Key { vk, shift, ..Default::default() }
    }

    fn page(m: &mut Ime) -> Vec<String> {
        let at = m.page_index() * m.per_page;
        m.cands()[at..].iter().map(|c| c.text.clone()).collect()
    }

    /// Presses a key the way TSF does: test first, key-down only when eaten.
    fn press(m: &mut Ime, k: Key) -> String {
        if !m.test(k) {
            assert!(m.down(k).is_none());
            return String::new();
        }
        m.down(k).unwrap().commit
    }

    /// Types `s`, returning what was committed; keys that pass through to the app show as `|c|`.
    fn typ(m: &mut Ime, s: &str) -> String {
        s.chars()
            .map(|c| {
                let k = key(c);
                if m.test(k) { m.down(k).unwrap().commit } else { format!("|{c}|") }
            })
            .collect()
    }

    #[test]
    fn pinyin_and_punctuation() {
        let mut m = ime(true);
        assert_eq!(typ(&mut m, "nihao "), "你好");
        assert_eq!(typ(&mut m, ",?!\\"), "，？！、");
        assert_eq!(typ(&mut m, "\"\"''"), "“”‘’");
        // after a digit . and , pass through as typed
        assert_eq!(typ(&mut m, "3.14,5."), "|3||.||1||4||,||5||.|");
        assert_eq!(typ(&mut m, "a."), "");
        assert_eq!(press(&mut m, key('\n')), "a");
        assert_eq!(typ(&mut m, "."), "。");
        // Shift+letter commits the first candidate, then the letter
        assert_eq!(typ(&mut m, "nihaoA"), "你好A");
        // other punctuation while composing works the same way
        assert_eq!(typ(&mut m, "nihao?"), "你好？");
        // Esc clears, Backspace edits
        assert_eq!(typ(&mut m, "ni"), "");
        press(&mut m, Key { vk: VK_ESCAPE, ..Default::default() });
        assert!(!m.engine.is_composing());
    }

    #[test]
    fn modes() {
        let mut m = ime(true);
        let shift = Key { vk: 0x10, shift: true, ..Default::default() };
        // Shift alone toggles; with a letter in between it doesn't
        assert!(!m.test(shift));
        assert_eq!(m.up(0x10).unwrap().hint, Some("英"));
        assert!(!m.mode.chinese);
        assert_eq!(typ(&mut m, "ni,"), "|n||i||,|");
        m.test(shift);
        typ(&mut m, "A");
        assert!(m.up(0x10).is_none());
        // Shift alone while composing commits the letters and switches
        m.mode.chinese = true;
        typ(&mut m, "ni");
        m.test(shift);
        assert_eq!(m.up(0x10).unwrap().commit, "ni");
        // full width, also in English mode
        assert!(m.test(Key { vk: VK_SPACE, shift: true, ..Default::default() }));
        assert_eq!(m.down(Key { vk: VK_SPACE, shift: true, ..Default::default() }).unwrap().hint, Some("全"));
        assert_eq!(typ(&mut m, "a1 "), "ａ１\u{3000}");
        // Ctrl+Shift+F: Traditional with Taiwan quotes and vocabulary
        m.set_mode(Mode { chinese: true, full: false, trad: false });
        let f = Key { vk: 0x46, shift: true, ctrl: true, ..Default::default() };
        assert!(m.test(f));
        assert_eq!(m.down(f).unwrap().hint, Some("繁"));
        assert_eq!(typ(&mut m, "ruanjian \"\"''"), "軟體「」『』");
        // toggle key Ctrl: Shift alone no longer toggles, Ctrl alone does, Ctrl+Shift+F doesn't
        m.configure(&Settings { toggle: Toggle::Ctrl, ..Default::default() });
        m.test(shift);
        assert!(m.up(0x10).is_none());
        let ctrl = Key { vk: 0x11, ctrl: true, ..Default::default() };
        m.test(ctrl);
        assert_eq!(m.up(0x11).unwrap().hint, Some("英"));
        m.test(ctrl);
        m.test(f);
        assert!(m.up(0x11).is_none());
        // toggle off: neither key toggles
        m.configure(&Settings { toggle: Toggle::Off, ..Default::default() });
        m.test(ctrl);
        assert!(m.up(0x11).is_none());
        m.test(shift);
        assert!(m.up(0x10).is_none());
    }

    #[test]
    fn paging_and_picks() {
        let mut m = ime(true);
        typ(&mut m, "shi");
        let first = page(&mut m);
        assert_eq!(first.len(), 7);
        typ(&mut m, "=");
        assert_eq!(m.page_index(), 1);
        let second = page(&mut m)[1].clone();
        assert!(!first.contains(&second));
        assert_eq!(typ(&mut m, "2"), second);
        typ(&mut m, "shi.-");
        assert_eq!(m.page_index(), 0);
        press(&mut m, Key { vk: VK_ESCAPE, ..Default::default() });
        // [ ] take one character of a word
        assert_eq!(typ(&mut m, "nihao]"), "好");
        // a pick that leaves pinyin keeps composing
        typ(&mut m, "nihaoshijie");
        let ni = page(&mut m).iter().position(|c| c == "你好").unwrap();
        assert_eq!(press(&mut m, key(char::from_digit(ni as u32 + 1, 10).unwrap())), "");
        assert!(m.engine.preedit().starts_with("你好"));
        // 5 per page (fresh memory): 6 picks nothing, the next page starts at the 6th candidate
        let mut m = ime(true);
        m.configure(&Settings { page: 5, ..Default::default() });
        typ(&mut m, "shi");
        assert_eq!(page(&mut m), first[..5]);
        assert_eq!(typ(&mut m, "6="), "");
        assert_eq!(page(&mut m)[0], first[5]);
    }

    #[test]
    fn adjust() {
        let mut m = ime(true);
        let [left, right, esc] = [VK_LEFT, VK_RIGHT, VK_ESCAPE].map(|vk| Key { vk, ..Default::default() });
        typ(&mut m, "nihaoshijie");
        press(&mut m, left);
        press(&mut m, left);
        let i = page(&mut m).iter().position(|c| c == "拟好").unwrap();
        assert_eq!(press(&mut m, key(char::from_digit(i as u32 + 1, 10).unwrap())), "");
        assert_eq!(m.engine.preedit(), "拟好世界");
        // Esc goes back to typing, a second Esc clears
        press(&mut m, esc);
        assert!(!m.engine.adjusting() && m.engine.is_composing());
        press(&mut m, esc);
        assert!(!m.engine.is_composing());
        // Space and Enter commit the sentence as shown, punctuation then itself
        for (k, want) in [(' ', "你好世界"), ('\n', "你好世界"), ('?', "你好世界？")] {
            typ(&mut m, "nihaoshijie");
            press(&mut m, left);
            assert_eq!(press(&mut m, key(k)), want);
        }
        // → past the last word goes back to typing; outside a composition arrows pass through
        typ(&mut m, "ni");
        press(&mut m, left);
        press(&mut m, right);
        assert!(!m.engine.adjusting());
        press(&mut m, esc);
        assert!(!m.test(left));
    }
}
