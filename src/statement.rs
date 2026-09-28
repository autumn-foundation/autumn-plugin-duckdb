//! Counts the statements in SQL text.
//!
//! `duckdb-rs` runs each statement before the last one in `prepare`. The plugin refuses SQL with more than one statement.
//!
//! # Contract
//!
//! - A `;` outside literals and comments ends a statement.
//! - A statement counts only if it has text other than white space and comments.
//! - Literals are `'...'` with `''`, `E'...'` with `\` escapes, `"..."` with `""` and dollar quotes.
//! - A dollar quote is `$$...$$` or `$tag$...$tag$`. A tag starts with a letter or `_`. `$1` is a parameter.
//! - Comments are `--` to the line end and `/* ... */`. Block comments nest.
//! - An open literal or comment continues to the end of the text.

/// Gives the number of statements in `sql`.
pub(crate) fn count(sql: &str) -> usize {
    let _ = sql;
    0
}

#[cfg(test)]
mod tests;
