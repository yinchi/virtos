//! How many screen columns a character takes -- the one implementation lives in `textlib::width`
//! (generated from the console's own font data, shared with `ls` and `column`); this re-exports it
//! under the editor's own module name. See there for the rules.

pub use textlib::width::{cell_width, is_zero_width, text_width};
