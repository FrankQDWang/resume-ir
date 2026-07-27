use super::*;

fn v32_connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE source_revision (
                 id TEXT PRIMARY KEY NOT NULL
             );
             CREATE TABLE resume_version (
                 id TEXT PRIMARY KEY NOT NULL,
                 source_revision_id TEXT NOT NULL,
                 parse_version TEXT NOT NULL
             );
             CREATE TABLE source_root (
                 id TEXT PRIMARY KEY NOT NULL,
                 state TEXT NOT NULL
             );
             CREATE TABLE source_occurrence (
                 root_id TEXT NOT NULL,
                 relative_path TEXT NOT NULL,
                 source_revision_id TEXT NOT NULL,
                 state TEXT NOT NULL,
                 PRIMARY KEY (root_id, relative_path)
             );",
        )
        .unwrap();
    connection
}

#[test]
fn pdf_reprocess_backfill_uses_a_bounded_lookup_and_leaves_no_schema_artifact() {
    let mut connection = v32_connection();
    connection
        .execute(
            "INSERT INTO source_root (id, state) VALUES ('root', 'active')",
            [],
        )
        .unwrap();
    for index in 0..256 {
        let revision = format!("revision-{index:03}");
        connection
            .execute("INSERT INTO source_revision (id) VALUES (?1)", [&revision])
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_occurrence (
                     root_id, relative_path, source_revision_id, state
                 ) VALUES ('root', ?1, ?2, 'present')",
                rusqlite::params![format!("resume-{index:03}.pdf"), revision],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO resume_version (
                     id, source_revision_id, parse_version
                 ) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    format!("version-{index:03}"),
                    revision,
                    if index % 2 == 0 {
                        PDFIUM_PARSER_CONTRACT
                    } else {
                        "parser-legacy"
                    }
                ],
            )
            .unwrap();
    }

    let transaction = connection.transaction().unwrap();
    transaction.execute_batch(schema_v33::SCHEMA).unwrap();
    create_pdf_reprocess_lookup_index(&transaction).unwrap();
    let mut plan = transaction
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT 1 FROM resume_version AS version
             WHERE version.source_revision_id = ?1
               AND version.parse_version = ?2",
        )
        .unwrap();
    let details = plan
        .query_map(
            rusqlite::params!["revision-001", PDFIUM_PARSER_CONTRACT],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains(PDF_REPROCESS_LOOKUP_INDEX)),
        "{details:?}"
    );
    drop(plan);
    transaction.rollback().unwrap();

    let transaction = connection.transaction().unwrap();
    apply_v32_to_v33(&transaction).unwrap();
    transaction.commit().unwrap();

    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pdf_reprocess_job", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        128
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                 WHERE type = 'index' AND name = ?1",
                [PDF_REPROCESS_LOOKUP_INDEX],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}
