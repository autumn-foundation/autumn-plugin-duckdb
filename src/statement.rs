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
pub(crate) fn count(sql: &str) -> Option<usize> {
    Some(count_old(sql))
}

fn count_old(sql: &str) -> usize {
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
            (b'/', Some(b'*')) => block_end(bytes, at),
            (b, _) if b.is_ascii_whitespace() => at + 1,
            _ => {
                has_text = true;
                token_end(bytes, at)
            }
        };
    }
    statements + usize::from(has_text)
}

/// Gives the end of the token at `at`. The token is not white space or a comment.
fn token_end(bytes: &[u8], at: usize) -> usize {
    let starts_word = at == 0 || bytes.get(at - 1).is_none_or(|&b| !is_word(b));
    match (bytes.get(at), bytes.get(at + 1)) {
        (Some(b'\''), _) => quote_end(bytes, at + 1, b'\'', false),
        (Some(b'"'), _) => quote_end(bytes, at + 1, b'"', false),
        (Some(b'E' | b'e'), Some(b'\'')) if starts_word => quote_end(bytes, at + 2, b'\'', true),
        (Some(b'$'), _) => dollar_end(bytes, at),
        (Some(&b), _) if is_word(b) => {
            let mut end = at;
            while bytes.get(end).is_some_and(|&b| is_word(b)) {
                end += 1;
            }
            end
        }
        _ => at + 1,
    }
}

/// Returns `true` for a byte of a word. Bytes of non-ASCII characters are word bytes.
const fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Gives the end of a quoted token. `from` is the first byte after the open quote.
fn quote_end(bytes: &[u8], from: usize, quote: u8, backslash: bool) -> usize {
    let mut at = from;
    while let Some(&byte) = bytes.get(at) {
        if backslash && byte == b'\\' {
            at += 2;
        } else if byte == quote {
            if bytes.get(at + 1) != Some(&quote) {
                return at + 1;
            }
            at += 2;
        } else {
            at += 1;
        }
    }
    bytes.len()
}

/// Gives the end of a dollar quote, or of a lone `$` such as a parameter sign.
fn dollar_end(bytes: &[u8], at: usize) -> usize {
    let mut tag_end = at + 1;
    if bytes
        .get(tag_end)
        .is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_')
    {
        while bytes
            .get(tag_end)
            .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
        {
            tag_end += 1;
        }
    }
    if bytes.get(tag_end) != Some(&b'$') {
        return at + 1;
    }
    let tag = &bytes[at..=tag_end];
    bytes[tag_end + 1..]
        .windows(tag.len())
        .position(|window| window == tag)
        .map_or(bytes.len(), |offset| tag_end + 1 + offset + tag.len())
}

/// Gives the end of a `--` comment: the byte after the line end.
fn line_end(bytes: &[u8], at: usize) -> usize {
    bytes[at..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |offset| at + offset + 1)
}

/// Gives the end of a `/* */` comment. Block comments nest.
fn block_end(bytes: &[u8], at: usize) -> usize {
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
                    return at;
                }
            }
            _ => at += 1,
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests;
