//! Keystroke latency: `cargo run --release -p engine --example bench` (after dictgen wrote target/dict.bin).

use engine::{Dict, Engine, Options, User};
use std::time::Instant;

fn main() {
    let t = Instant::now();
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/dict.bin")).expect("run dictgen first");
    let dict: &'static Dict = Box::leak(Box::new(Dict::load(Box::leak(bytes.into_boxed_slice())).unwrap()));
    println!("load {:?}", t.elapsed());
    for trad in [false, true] {
        let mut e = Engine::new(dict, User::open(None, true), Options { trad, ..Default::default() });
        let (mut n, mut sum, mut max) = (0, 0.0f64, 0.0f64);
        for py in ["woshizhongguoren", "jintiantianqizhenhaowomenyiqiquwanba", "zgrmjfj", "s", "zhongguorenmindaxuejisuanjixueyuan"] {
            e.clear();
            for c in py.chars() {
                let t = Instant::now();
                e.push(c);
                e.candidates(7);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                (n, sum, max) = (n + 1, sum + ms, max.max(ms));
            }
            println!("{py}: {}", e.candidates(1).first().map_or("", |c| &c.text));
        }
        println!("trad={trad}: {n} keys, avg {:.3} ms, max {:.3} ms", sum / n as f64, max);
    }
}
