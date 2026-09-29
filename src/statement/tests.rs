use proptest::prelude::*;

use super::*;

#[test]
fn one_statement_counts_once() {
    assert_eq!(count("SELECT 1"), 1);
    assert_eq!(count("SELECT 1;"), 1);
    assert_eq!(count("  SELECT 1 ;  \n"), 1);
}

#[test]
fn empty_text_has_no_statements() {
    assert_eq!(count(""), 0);
    assert_eq!(count(" ; ;\n"), 0);
    assert_eq!(count("-- only a comment"), 0);
    assert_eq!(count("/* only */ ; /* comments */"), 0);
}

#[test]
fn semicolons_separate_statements() {
    assert_eq!(count("SELECT 1; SELECT 2"), 2);
    assert_eq!(count("SELECT 1;;SELECT 2;"), 2);
    assert_eq!(count("DELETE FROM t; SELECT 1"), 2);
}

#[test]
fn semicolons_in_literals_do_not_count() {
    assert_eq!(count("SELECT ';'"), 1);
    assert_eq!(count("SELECT 'it''s; ok'"), 1);
    assert_eq!(count(r"SELECT E'\'; x'"), 1);
    assert_eq!(count(r"SELECT e'a\\'; SELECT 2"), 2);
    assert_eq!(count(r#"SELECT 1 AS "a;b""#), 1);
    assert_eq!(count(r#"SELECT 1 AS "a"";b""#), 1);
    assert_eq!(count("SELECT $$;$$"), 1);
    assert_eq!(count("SELECT $t$ $$; $t$"), 1);
    assert_eq!(count("SELECT $_x1$;$_x1$"), 1);
}

#[test]
fn a_backslash_does_not_escape_in_a_plain_string() {
    assert_eq!(count(r"SELECT '\'; SELECT 2"), 2);
}

#[test]
fn the_e_prefix_needs_a_word_start() {
    // `somee'...'` is not an escape string.
    assert_eq!(count(r"SELECT 1 AS somee, 'a\'; SELECT 2"), 2);
}

#[test]
fn parameters_are_not_dollar_quotes() {
    assert_eq!(count("SELECT $1; SELECT $2"), 2);
    assert_eq!(count("SELECT $1, $name; SELECT 2"), 2);
}

#[test]
fn semicolons_in_comments_do_not_count() {
    assert_eq!(count("SELECT 1 -- ; SELECT 2"), 1);
    assert_eq!(count("SELECT 1 /* ; */"), 1);
    assert_eq!(count("SELECT 1 /* /* ; */ ; */"), 1);
    assert_eq!(count("SELECT 1 -- x\n; SELECT 2"), 2);
}

#[test]
fn an_open_literal_or_comment_continues_to_the_end() {
    assert_eq!(count("SELECT '; SELECT 2"), 1);
    assert_eq!(count("SELECT /* ; SELECT 2"), 1);
    assert_eq!(count("SELECT $$; SELECT 2"), 1);
}

#[test]
fn non_ascii_text_is_safe() {
    assert_eq!(count("SELECT 'é;ü'; SELECT '日本'"), 2);
    assert_eq!(count("SELECT $é$;"), 1);
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
];

/// Separators between statements.
const SEPARATORS: &[&str] = &[";", " ; ", ";\n", "; ;", ";\n-- c\n;", "; /* c */ ;"];

thread_local! {
    /// One oracle database for each test thread.
    static ORACLE: duckdb::Connection = {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE log (v VARCHAR)").unwrap();
        conn
    };
}

/// Gives the number of statements that DuckDB runs for `sql`.
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
        prop_assert_eq!(count(&sql), duckdb_count(&sql), "{}", sql);
    }
}
