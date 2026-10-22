//! The username upgrade preserves existing accounts and active credentials.
use sqlx::{Connection, Row};
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn username_upgrade_preserves_users_and_sessions_and_resolves_prefix_collisions() {
    let url = std::env::var("OSTOOL_TEST_DATABASE_URL").unwrap();
    let mut connection = sqlx::PgConnection::connect(&url).await.unwrap();
    let schema = format!("ostool_username_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('search_path',$1,false)")
        .bind(format!("{schema},public"))
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/20261022000000_initial_schema.sql"
    ))
    .execute(&mut connection)
    .await
    .unwrap();
    for email in [
        "admin@example.com",
        "same@first.test",
        "same@second.test",
        "invalid.prefix@example.com",
    ] {
        sqlx::query("INSERT INTO users(email,password_hash) VALUES($1,'existing-hash')")
            .bind(email)
            .execute(&mut connection)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO oauth_sessions(user_id,client_id,access_token_hash,refresh_token_hash,access_expires_at,refresh_expires_at,absolute_expires_at) SELECT id,'web','access-hash','refresh-hash',now()+interval '1 hour',now()+interval '1 day',now()+interval '1 day' FROM users WHERE email='admin@example.com'").execute(&mut connection).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/20261022010000_global_usernames.sql"
    ))
    .execute(&mut connection)
    .await
    .unwrap();
    let users = sqlx::query("SELECT email,username,password_hash FROM users ORDER BY email")
        .fetch_all(&mut connection)
        .await
        .unwrap();
    assert_eq!(users.len(), 4);
    let mut names = std::collections::BTreeSet::new();
    for user in users {
        let name: String = user.get("username");
        assert!(names.insert(name.clone()));
        assert_eq!(user.get::<String, _>("password_hash"), "existing-hash");
        if user.get::<String, _>("email") == "admin@example.com" {
            assert_eq!(name, "admin");
        } else {
            assert!(name.starts_with("user_"));
        }
    }
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT revoked_at IS NULL FROM oauth_sessions")
            .fetch_one(&mut connection)
            .await
            .unwrap()
    );
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&mut connection)
        .await
        .unwrap();
}
