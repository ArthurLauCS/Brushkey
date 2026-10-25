//! The user's settings: written by the settings app to `%APPDATA%\Brushkey\settings`
//! as `key=value` lines, read by the IME on activation and whenever an app gets focus.

use crate::user_dir;
use std::fmt;

/// The key that toggles 中/英 when pressed and released alone.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Toggle {
    #[default]
    Shift,
    Ctrl,
    Off,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub toggle: Toggle,
    pub vertical: bool,
    /// Candidates per page, 3-9 (picked with 1-9).
    pub page: usize,
    /// Candidate text size in DIPs, 12-36.
    pub size: u32,
    pub font: String,
    pub font_trad: String,
    /// Bit i lets `engine::FUZZY[i]` feed the second candidate; by default the
    /// nasal pairs and z/zh, c/ch, s/sh.
    pub fuzzy: u32,
    /// Language of the settings app; None follows Windows.
    pub english: Option<bool>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            toggle: Toggle::Shift,
            vertical: false,
            page: 7,
            size: 18,
            font: "Microsoft YaHei UI".into(),
            font_trad: "Microsoft JhengHei UI".into(),
            fuzzy: 0b111_1100_0111,
            english: None,
        }
    }
}

impl Settings {
    /// Missing or unreadable values keep their defaults; numbers are kept in range.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for (k, v) in text.lines().filter_map(|l| l.split_once('=')) {
            let n = v.parse::<u32>().ok();
            match k {
                "toggle" => s.toggle = [Toggle::Shift, Toggle::Ctrl, Toggle::Off].into_iter().find(|t| t.name() == v).unwrap_or_default(),
                "vertical" => s.vertical = v == "1",
                "page" => s.page = n.map_or(s.page, |n| n.clamp(3, 9) as usize),
                "size" => s.size = n.map_or(s.size, |n| n.clamp(12, 36)),
                "font" if !v.is_empty() => s.font = v.into(),
                "font_trad" if !v.is_empty() => s.font_trad = v.into(),
                "fuzzy_pairs" => s.fuzzy = n.unwrap_or(s.fuzzy),
                "lang" => s.english = match v { "en" => Some(true), "zh" => Some(false), _ => None },
                _ => {}
            }
        }
        s
    }

    pub fn load() -> Settings {
        user_dir().and_then(|d| std::fs::read_to_string(d.join("settings")).ok()).map_or_else(Settings::default, |t| Settings::parse(&t))
    }

    pub fn save(&self) {
        if let Some(d) = user_dir() {
            let _ = std::fs::write(d.join("settings"), self.to_string());
        }
    }
}

impl Toggle {
    fn name(self) -> &'static str {
        ["shift", "ctrl", "off"][self as usize]
    }
}

impl fmt::Display for Settings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let lang = match self.english { Some(true) => "en", Some(false) => "zh", None => "" };
        writeln!(f, "toggle={}\nvertical={}", self.toggle.name(), self.vertical as u8)?;
        writeln!(f, "page={}\nsize={}\nfont={}\nfont_trad={}", self.page, self.size, self.font, self.font_trad)?;
        writeln!(f, "fuzzy_pairs={}\nlang={lang}", self.fuzzy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_ranges() {
        let s = Settings { toggle: Toggle::Ctrl, vertical: true, page: 5, size: 24, font: "KaiTi".into(), font_trad: "PMingLiU".into(), fuzzy: 0b101, english: Some(true) };
        assert_eq!(Settings::parse(&s.to_string()), s);
        assert_eq!(Settings::parse(""), Settings::default());
        let t = Settings::parse("page=0\nsize=99\ntoggle=x\nfont=\nfuzzy_pairs=a");
        let d = Settings::default();
        assert_eq!((t.page, t.size, t.toggle, t.font, t.fuzzy), (3, 36, Toggle::Shift, d.font, d.fuzzy));
    }
}
