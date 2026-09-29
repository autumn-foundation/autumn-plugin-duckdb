use proptest::prelude::*;

use super::*;

#[test]
fn one_statement_counts_once() {
    assert_eq!(count("SELECT 1"), Some(1));
    assert_eq!(count("SELECT 1;"), Some(1));
    assert_eq!(count("  SELECT 1 ;  \n"), Some(1));
}

#[test]
fn empty_text_has_no_statements() {
    assert_eq!(count(""), Some(0));
    assert_eq!(count(" ; ;\n"), Some(0));
    assert_eq!(count("-- only a comment"), Some(0));
    assert_eq!(count("/* only */ ; /* comments */"), Some(0));
}

#[test]
fn semicolons_separate_statements() {
    assert_eq!(count("SELECT 1; SELECT 2"), Some(2));
    assert_eq!(count("SELECT 1;;SELECT 2;"), Some(2));
    assert_eq!(count("DELETE FROM t; SELECT 1"), Some(2));
}

#[test]
fn semicolons_in_literals_do_not_count() {
    assert_eq!(count("SELECT ';'"), Some(1));
    assert_eq!(count("SELECT 'it''s; ok'"), Some(1));
    assert_eq!(count(r"SELECT E'\'; x'"), Some(1));
    assert_eq!(count(r"SELECT e'a\\'; SELECT 2"), Some(2));
    assert_eq!(count(r#"SELECT 1 AS "a;b""#), Some(1));
    assert_eq!(count(r#"SELECT 1 AS "a"";b""#), Some(1));
    assert_eq!(count("SELECT $$;$$"), Some(1));
    assert_eq!(count("SELECT $t$ $$; $t$"), Some(1));
    assert_eq!(count("SELECT $_x1$;$_x1$"), Some(1));
}

#[test]
fn a_backslash_does_not_escape_in_a_plain_string() {
    assert_eq!(count(r"SELECT '\'; SELECT 2"), Some(2));
}

#[test]
fn the_e_prefix_needs_a_word_start() {
    // `somee'...'` is not an escape string.
    assert_eq!(count(r"SELECT 1 AS somee, 'a\'; SELECT 2"), Some(2));
}

#[test]
fn parameters_are_not_dollar_quotes() {
    assert_eq!(count("SELECT $1; SELECT $2"), Some(2));
    assert_eq!(count("SELECT $1, $name; SELECT 2"), Some(2));
}

#[test]
fn semicolons_in_comments_do_not_count() {
    assert_eq!(count("SELECT 1 -- ; SELECT 2"), Some(1));
    assert_eq!(count("SELECT 1 /* ; */"), Some(1));
    assert_eq!(count("SELECT 1 /* /* ; */ ; */"), Some(1));
    assert_eq!(count("SELECT 1 -- x\n; SELECT 2"), Some(2));
}

#[test]
fn an_open_literal_or_block_comment_gives_none() {
    assert_eq!(count("SELECT '; SELECT 2"), None);
    assert_eq!(count("SELECT /* ; SELECT 2"), None);
    assert_eq!(count("SELECT $$; SELECT 2"), None);
    assert_eq!(count(r#"SELECT "a; SELECT 2"#), None);
    assert_eq!(count(r"SELECT E'\"), None);
}

#[test]
fn an_open_line_comment_at_the_end_is_normal() {
    assert_eq!(count("SELECT 1 -- note"), Some(1));
}

#[test]
fn a_carriage_return_ends_a_line_comment() {
    assert_eq!(
        count("SELECT 1 --\r; CREATE TABLE m (i INT); SELECT 2"),
        Some(3)
    );
}

#[test]
fn a_dollar_sign_inside_a_word_is_part_of_the_word() {
    assert_eq!(
        count("SELECT 1 AS a$x$; CREATE TABLE m (i INT); SELECT 1 AS b$x$"),
        Some(3)
    );
    assert_eq!(count("SELECT $1; SELECT a$1"), Some(2));
}

#[test]
fn dollar_quote_tags_can_have_non_ascii_letters() {
    assert_eq!(
        count("SELECT $é$'$é$; CREATE TABLE m (i INT); SELECT '1'"),
        Some(3)
    );
    assert_eq!(count("SELECT $é$;$é$"), Some(1));
}

#[test]
fn non_ascii_text_is_safe() {
    assert_eq!(count("SELECT 'é;ü'; SELECT '日本'"), Some(2));
    assert_eq!(count("SELECT $é$;"), None);
}

/// Expressions with `;`, quotes and comments. Each one is valid in DuckDB.
const EXPRESSIONS: &[&str] = &[
    "1",
    "';'",
    "'it''s;'",
    r"E'\';'",
    r"e'\\'",
    "$$;$$",
    "$t$;$$;$t$",
    r#"(SELECT 1 AS "a;b")"#,
    r#"(SELECT 1 AS "a"";")"#,
    "/* ; */ 2",
    "/* /* ; */ ; */ 3",
    "-- ;\n 4",
    "'--;'",
    "'/*;'",
    "-- ;\r 5",
    "(SELECT 1 AS a$x$)",
    "(SELECT 1 AS \"a$$;\")",
    "$é$;'$é$",
    "$_é1$;$$$_é1$",
    "e'\\\\'",
    "(SELECT 1 AS ée)",
    "\t6\x0c",
];

/// Separators between statements.
const SEPARATORS: &[&str] = &[
    ";",
    " ; ",
    ";\n",
    "; ;",
    ";\n-- c\n;",
    "; /* c */ ;",
    ";\r\n",
    "\t;\x0c",
    "; -- x\r;",
];

thread_local! {
    /// One oracle database for each test thread.
    static ORACLE: duckdb::Connection = {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE log (v VARCHAR)").unwrap();
        conn
    };
}

/// Returns the number of statements that DuckDB runs for `sql`.
fn duckdb_count(sql: &str) -> usize {
    ORACLE.with(|conn| {
        conn.execute_batch("DELETE FROM log").unwrap();
        // `prepare` runs each statement before the last one.
        conn.prepare(sql).unwrap().execute([]).unwrap();
        conn.query_row("SELECT count(*) FROM log", [], |row| row.get(0))
            .unwrap()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn the_count_equals_the_duckdb_count(
        parts in prop::collection::vec(
            (prop::sample::select(EXPRESSIONS), prop::sample::select(SEPARATORS)),
            1..5,
        ),
        trailing in any::<bool>(),
    ) {
        let mut sql = String::new();
        for (index, (expression, separator)) in parts.iter().enumerate() {
            if index > 0 {
                sql.push_str(separator);
            }
            sql.push_str("INSERT INTO log SELECT ");
            sql.push_str(expression);
        }
        if trailing {
            sql.push_str(";\n");
        }
        prop_assert_eq!(count(&sql), Some(duckdb_count(&sql)), "{}", sql);
    }
}

/// Returns the number of `INSERT INTO log` statements that DuckDB runs for `sql`.
///
/// A parse error runs nothing. A later error can come after earlier statements ran.
fn duckdb_side_effects(sql: &str) -> usize {
    ORACLE.with(|conn| {
        conn.execute_batch("DELETE FROM log").unwrap();
        if let Ok(mut stmt) = conn.prepare(sql) {
            let _ = stmt.execute([]);
        }
        conn.query_row("SELECT count(*) FROM log", [], |row| row.get(0))
            .unwrap()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Safety: the count is never lower than the statements that DuckDB runs.
    #[test]
    fn the_count_never_misses_a_statement_that_duckdb_runs(
        fragments in prop::collection::vec("[ '\"$aEe\\\\/*;\\n\\r\\t\\-é1x]{0,8}", 1..4),
    ) {
        let sql = fragments
            .iter()
            .map(|fragment| format!("INSERT INTO log SELECT 1 {fragment}"))
            .collect::<Vec<_>>()
            .join(";");
        let ran = duckdb_side_effects(&sql);
        if let Some(counted) = count(&sql) {
            prop_assert!(counted >= ran, "counted {} but DuckDB ran {}: {:?}", counted, ran, sql);
        }
    }
}
