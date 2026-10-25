//! dictgen <data dir> <out file>: compiles the dictionary sources into the file the IME maps.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let [_, data, out] = &a[..] else {
        eprintln!("usage: dictgen <data dir> <out file>");
        std::process::exit(2);
    };
    let bytes = engine::Dict::compile(data.as_ref()).expect("read dictionary sources");
    std::fs::write(out, &bytes).expect("write dictionary");
    println!("{out}: {} bytes", bytes.len());
}
