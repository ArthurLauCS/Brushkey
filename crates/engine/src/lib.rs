//! Pinyin input engine: dictionary, sentence conversion, user memory and
//! Traditional (Taiwan) conversion. Pure std, so it builds and tests anywhere.

mod dict;
mod engine;
mod table;
mod trad;
mod user;

pub use dict::Dict;
pub use engine::{Cand, Engine, FUZZY, Options};
pub use user::User;
