pub mod migrate;
pub mod queries;
pub mod scheme;

use sqlx::postgres::{PgPoolOptions, Postgres};

pub type Pool = sqlx::Pool<Postgres>;

pub async fn create_db_pool(conn_string: &str) -> Result<Pool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(conn_string).await?;

    Ok(pool)
}