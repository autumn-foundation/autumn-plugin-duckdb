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
//! - A word starts with a letter or `_`. Then it can have digits and `$`. Non-ASCII characters are letters.
//! - Comments are `--` to the line end and `/* ... */`. A line feed or a carriage return ends a line.
//!   Block comments nest.
//! - Text that ends in an open literal or block comment gives `None`. The plugin refuses it.

/// Gives the number of statements in `sql`.
///
/// Gives `None` if the text ends in an open literal or block comment.
pub(crate) fn count(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut statements = 0;
    let mut has_text = false;
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let next = bytes.get(at + 1).copied();
        at = match (byte, next) {
            (b';', _) => {
                statements += usize::from(has_text);
                has_text = false;
                at + 1
            }
            (b'-', Some(b'-')) => line_end(bytes, at),
            (b'/', Some(b'*')) => block_end(bytes, at)?,
            (b, _) if b.is_ascii_whitespace() => at + 1,
            _ => {
                has_text = true;
                token_end(bytes, at)?
            }
        };
    }
    Some(statements + usize::from(has_text))
}

/// Gives the end of the token at `at`. The token is not white space or a comment.
fn token_end(bytes: &[u8], at: usize) -> Option<usize> {
    match (bytes.get(at), bytes.get(at + 1)) {
        (Some(b'\''), _) => quote_end(bytes, at + 1, b'\'', false),
        (Some(b'"'), _) => quote_end(bytes, at + 1, b'"', false),
        (Some(b'E' | b'e'), Some(b'\'')) => quote_end(bytes, at + 2, b'\'', true),
        (Some(b'$'), _) => dollar_end(bytes, at),
        (Some(&b), _) if is_word_start(b) => {
            let mut end = at + 1;
            while bytes.get(end).is_some_and(|&b| is_word(b) || b == b'$') {
                end += 1;
            }
            Some(end)
        }
        _ => Some(at + 1),
    }
}

/// Returns `true` for the first byte of a word. Bytes of non-ASCII characters are letters.
const fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

/// Returns `true` for a byte of a word or a dollar-quote tag.
const fn is_word(byte: u8) -> bool {
    is_word_start(byte) || byte.is_ascii_digit()
}

/// Gives the end of a quoted token. `from` is the first byte after the open quote.
fn quote_end(bytes: &[u8], from: usize, quote: u8, backslash: bool) -> Option<usize> {
    let mut at = from;
    while let Some(&byte) = bytes.get(at) {
        if backslash && byte == b'\\' {
            at += 2;
        } else if byte == quote {
            if bytes.get(at + 1) != Some(&quote) {
                return Some(at + 1);
            }
            at += 2;
        } else {
            at += 1;
        }
    }
    None
}

/// Gives the end of a dollar quote, or of a lone `$` such as a parameter sign.
fn dollar_end(bytes: &[u8], at: usize) -> Option<usize> {
    let mut tag_end = at + 1;
    if bytes.get(tag_end).is_some_and(|&b| is_word_start(b)) {
        while bytes.get(tag_end).is_some_and(|&b| is_word(b)) {
            tag_end += 1;
        }
    }
    if bytes.get(tag_end) != Some(&b'$') {
        return Some(at + 1);
    }
    let tag = &bytes[at..=tag_end];
    bytes[tag_end + 1..]
        .windows(tag.len())
        .position(|window| window == tag)
        .map(|offset| tag_end + 1 + offset + tag.len())
}

/// Gives the end of a `--` comment: the byte after the line end. `\n` and `\r` end a line.
fn line_end(bytes: &[u8], at: usize) -> usize {
    bytes[at..]
        .iter()
        .position(|&b| b == b'\n' || b == b'\r')
        .map_or(bytes.len(), |offset| at + offset + 1)
}

/// Gives the end of a `/* */` comment. Block comments nest.
fn block_end(bytes: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut at = at;
    while at < bytes.len() {
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(b'/'), Some(b'*')) => {
                depth += 1;
                at += 2;
            }
            (Some(b'*'), Some(b'/')) => {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => at += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests;
