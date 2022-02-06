pub mod consts;
pub mod migrate;
pub mod queries;

pub use self::consts::Pool;
use sqlx::postgres::PgPoolOptions;

pub async fn create_db_pool(conn_string: &str) -> Result<Pool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(conn_string)
        .await?;

    sqlx::migrate!("../kollider-hedge-db/migrations").run(&pool).await?;

    Ok(pool)
}
