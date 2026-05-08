use lightai_server::db;

#[tokio::test]
async fn sqlite_foreign_keys_are_enforced() {
    let dir = std::env::temp_dir().join(format!("lightai-fk-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let database_url = format!("sqlite://{}", dir.join("fk.db").display());
    let pool = db::connect(&database_url).await.unwrap();

    let result = sqlx::query(
        r#"
        INSERT INTO model_files (
            id, model_id, node_id, path, status, created_at, updated_at
        )
        VALUES ('file-a', 'missing-model', 'missing-node', '/models/a.gguf', 'verified', 1, 1)
        "#,
    )
    .execute(&pool)
    .await;

    assert!(result.is_err(), "foreign key violation should fail");
    let _ = std::fs::remove_dir_all(dir);
}
