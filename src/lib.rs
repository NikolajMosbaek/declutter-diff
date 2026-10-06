//! declutter: review a diff with its comments and tests shown, hidden, or on their own.
//!
//! Each changed file is parsed with tree-sitter, its comments are located, and
//! both versions are *projected* — comments removed, or everything but comments
//! removed — before diffing. Diffing the projections, rather than hiding lines of
//! a normal diff afterwards, is what makes comment-only hunks disappear and lets a
//! line that changed code and a trailing comment show only the code change.
//!
//! Tests are a file-level layer: a changed file is test code by path convention or
//! by importing a test framework, and is listed or left out as a whole.

pub mod classify;
pub mod diff;
pub mod editor;
pub mod git;
pub mod highlight;
pub mod lang;
pub mod moves;
pub mod palette;
pub mod pr;
pub mod project;
pub mod render;
pub mod review;
pub mod store;
pub mod target;
pub mod test_files;
pub mod tui;
