use engine::{Dict, Engine, Options, User};
use std::{path::PathBuf, sync::OnceLock};

fn dict() -> &'static Dict {
    static D: OnceLock<&'static Dict> = OnceLock::new();
    D.get_or_init(|| {
        let b = Dict::compile(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data").as_ref()).unwrap();
        Box::leak(Box::new(Dict::load(Box::leak(b.into_boxed_slice())).unwrap()))
    })
}

fn log(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("engine-test-{}-{name}.log", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn eng(path: Option<PathBuf>, trad: bool) -> Engine {
    Engine::new(dict(), User::open(path, true), Options { trad, ..Default::default() })
}

/// Types `py` from scratch and returns the first candidates.
fn typ(e: &mut Engine, py: &str) -> Vec<String> {
    e.clear();
    py.chars().for_each(|c| e.push(c));
    e.candidates(8).iter().map(|c| c.text.clone()).collect()
}

fn pick(e: &mut Engine, text: &str) -> Option<String> {
    let i = e.candidates(300).iter().position(|c| c.text == text);
    let Some(i) = i else {
        let names: Vec<_> = e.candidates(20).iter().map(|c| &c.text).collect();
        panic!("{text} not in {names:?}")
    };
    e.select(i)
}

#[test]
fn first_candidate() {
    let mut e = eng(None, false);
    let mut bad = vec![];
    for (py, want) in [
        ("nihao", "你好"),
        ("woshizhongguoren", "我是中国人"),
        ("jintiantianqizhenhao", "今天天气真好"),
        ("women'yiqiqudaguai", "我们一起去打怪"),
        ("xi'an", "西安"),
        ("fangan", "方案"),
        ("zhongg", "中国"),
        ("lvse", "绿色"),
        ("lue", "略"),
    ] {
        let got = typ(&mut e, py);
        if got.first().map(String::as_str) != Some(want) {
            bad.push(format!("{py}: want {want}, got {got:?}"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
    assert_ne!(typ(&mut e, "xian")[0], "西安");
    // Initials only rank by corpus frequency (这个 is ahead of 中国), so just the first page.
    assert!(typ(&mut e, "zg").contains(&"中国".into()));
    assert!(typ(&mut e, "nih").contains(&"你好".into()));
    // The first character comes before rare words for all of the pinyin.
    assert!(typ(&mut e, "woshi").contains(&"我".into()));
    assert!(typ(&mut e, "yixia").contains(&"一".into()));
}

#[test]
fn preedit_and_backspace() {
    let mut e = eng(None, false);
    typ(&mut e, "nihaoshijie");
    assert_eq!(e.preedit(), "ni'hao'shi'jie");
    assert_eq!(pick(&mut e, "你好"), None);
    assert_eq!(e.preedit(), "你好shi'jie");
    "shijie".chars().for_each(|_| e.pop());
    assert_eq!(e.preedit(), "你好");
    e.pop(); // undoes the pick
    assert_eq!(e.preedit(), "ni'hao");
    assert_eq!(e.commit_raw(), "nihao");
    assert!(!e.is_composing());
}

#[test]
fn memory() {
    let p = log("memory");
    let mut e = eng(Some(p.clone()), false);
    let top = typ(&mut e, "shi")[0].clone();
    assert_eq!(pick(&mut e, "事").as_deref(), Some("事"));
    assert_eq!(typ(&mut e, "shi")[0], "事", "one pick moves a word first");

    // Another process sees it on its next composition.
    let mut other = eng(Some(p.clone()), false);
    assert_eq!(typ(&mut other, "shi")[0], "事");
    typ(&mut e, "shi");
    pick(&mut e, "式");
    assert_eq!(typ(&mut other, "shi")[..2], ["式", "事"], "newest pick first");

    // Separate picks become one word.
    typ(&mut e, "maodou");
    assert_eq!(pick(&mut e, "猫"), None);
    assert_eq!(pick(&mut e, "豆").as_deref(), Some("猫豆"));
    assert_eq!(typ(&mut other, "maodou")[0], "猫豆");

    // Forgetting (Ctrl+Shift+number) and clearing the log.
    typ(&mut e, "shi");
    e.forget(0);
    e.forget(0);
    assert_eq!(typ(&mut e, "shi")[0], top);

    // A password field picks without learning.
    e.set_options(Options { private: true, ..e.options() });
    pick(&mut e, "事");
    assert_eq!(typ(&mut e, "shi")[0], top);
    std::fs::write(&p, "").unwrap();
    assert_ne!(typ(&mut other, "maodou")[0], "猫豆");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn memory_order() {
    let p = log("order");
    let mut e = eng(Some(p.clone()), false);
    typ(&mut e, "le");
    pick(&mut e, "乐");
    assert_eq!(typ(&mut e, "le")[0], "乐");
    typ(&mut e, "keyi");
    pick(&mut e, "可疑");
    assert_eq!(typ(&mut e, "keyi")[0], "可疑");
    // One pick of a rare homophone leaves sentences alone.
    assert_eq!(typ(&mut e, "meiyouwentile")[0], "没有问题了");
    assert_eq!(typ(&mut e, "keyibangwoma")[0], "可以帮我吗");

    // Initials match several learned words: the newest pick comes first.
    typ(&mut e, "shezhi");
    pick(&mut e, "设置");
    typ(&mut e, "sz");
    pick(&mut e, "实战");
    assert_eq!(typ(&mut e, "sz")[0], "实战");
    pick(&mut e, "设置");
    assert_eq!(typ(&mut e, "sz")[0], "设置");

    // A log full of repeats is rewritten with one line per word, order kept.
    std::fs::write(&p, "+\tshi\t事\n".repeat(10_010) + "+\tshi\t式\n").unwrap();
    let mut e = eng(Some(p.clone()), false);
    typ(&mut e, "le");
    pick(&mut e, "乐");
    assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 3);
    assert_eq!(typ(&mut e, "shi")[..2], ["式", "事"]);
    assert_eq!(typ(&mut eng(Some(p.clone()), false), "shi")[..2], ["式", "事"]);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn traditional() {
    let p = log("trad");
    let mut e = eng(Some(p.clone()), true);
    assert_eq!(typ(&mut e, "ruanjian")[..2], ["軟體", "軟件"]);
    let v = typ(&mut e, "shipin");
    assert!(v.iter().position(|s| s == "影片") < v.iter().position(|s| s == "視頻"), "{v:?}");
    let v = typ(&mut e, "gan");
    assert!(v.contains(&"乾".into()) && v.contains(&"幹".into()), "{v:?}");
    assert_eq!(typ(&mut e, "woderuanjianhuaile")[0], "我的軟體壞了");
    assert_eq!(typ(&mut e, "toufa")[0], "頭髮");

    typ(&mut e, "ruanjian");
    pick(&mut e, "軟件");
    assert_eq!(typ(&mut e, "ruanjian")[0], "軟件", "the Traditional form picked comes first");
    let mut simp = eng(Some(p.clone()), false);
    assert_eq!(typ(&mut simp, "ruanjian")[0], "软件");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn fuzzy_and_char_of_word() {
    // The best reading through the enabled pairs goes second; a pick of it is
    // learned as typed.
    let p = log("fuzzy");
    let mut e = Engine::new(dict(), User::open(Some(p.clone()), true), Options { fuzzy: 1 | 1 << 2 | 1 << 8, ..Default::default() }); // z = zh, s = sh, in = ing
    assert_eq!(typ(&mut e, "zongguo")[1], "中国");
    assert_eq!(typ(&mut e, "si")[1], "是");
    assert_eq!(typ(&mut e, "jinglai")[1], "进来");
    pick(&mut e, "进来");
    assert_eq!(typ(&mut e, "jinglai")[0], "进来");
    assert_eq!(typ(&mut eng(None, false), "zongguo")[1], "总");
    let _ = std::fs::remove_file(&p);

    let mut e = eng(None, false);
    typ(&mut e, "wenzhang");
    let i = e.candidates(50).iter().position(|c| c.text == "文章").unwrap();
    assert_eq!(e.select_char(i, true).as_deref(), Some("章"));
}

#[test]
fn swapped_letters() {
    let p = log("swap");
    let mut e = eng(Some(p.clone()), false);
    for (py, want) in [("zhogn", "中"), ("xainzai", "现在"), ("womne", "我们"), ("nhiao", "你好"), ("woxainzaizaijia", "我现在在家")] {
        assert_eq!(typ(&mut e, py)[0], want, "{py}");
    }
    // Letters that split into whole syllables are read as typed.
    assert!(!typ(&mut e, "gou").contains(&"国".into()));
    // The composition shows the letters as typed; a pick is learned as corrected.
    typ(&mut e, "xainzai");
    assert_eq!(e.preedit(), "xain'zai");
    pick(&mut e, "先在");
    assert_eq!(typ(&mut e, "xianzai")[0], "先在");
    let _ = std::fs::remove_file(&p);
}
