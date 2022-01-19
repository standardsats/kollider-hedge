use sqlx::postgres::Postgres;

pub type Pool = sqlx::Pool<Postgres>;
