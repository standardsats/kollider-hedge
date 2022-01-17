use super::consts::Pool;
use chrono::prelude::*;
use futures::StreamExt;
use kollider_hedge_domain::state::*;
use kollider_hedge_domain::update::*;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Failed to decode body by tag: {0}")]
    UpdateBody(#[from] UpdateBodyError),
    #[error("Failed to decode/encode JSON: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("Failed to reconstruct state: {0}")]
    StateInvalid(#[from] StateUpdateErr),
}

/// Alias for a `Result` with the error type `self::Error`.
pub type Result<T> = std::result::Result<T, Error>;

/// Query all history of updates until we hit a snapshot or the begining of time
pub async fn query_updates(pool: &Pool) -> Result<Vec<StateUpdate>> {
    let mut conn = pool.acquire().await?;
    let res = sqlx::query!("select * from updates order by created desc")
        .fetch(&mut conn)
        .fuse();
    futures::pin_mut!(res);

    let mut parsed: Vec<StateUpdate> = vec![];
    loop {
        let item = futures::select! {
            mmrow = res.next() => {
                if let Some(mrow) = mmrow {
                    let r = mrow?;
                    let body = UpdateTag::from_tag(&r.tag, r.version as u16, r.body.clone())?;
                    StateUpdate {
                        created: r.created,
                        body
                    }
                } else {
                    break;
                }
            },
            complete => break,
        };
        let is_end = item.body.tag() == UpdateTag::Snapshot;
        parsed.push(item);
        if is_end {
            break;
        }
    }
    Ok(parsed)
}

/// Insert new update in the chain of updates in database
pub async fn insert_update(pool: &Pool, update: UpdateBody) -> Result<()> {
    let now = Utc::now().naive_utc();
    let tag = format!("{}", update.tag());
    let body = update.json()?;
    sqlx::query!(
        "insert into updates (created, version, tag, body) values ($1, $2, $3, $4)",
        now,
        CURRENT_BODY_VERSION as i16,
        tag,
        body
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Reconstruct state from chain of updates and snapshots in the database
pub async fn query_state(pool: &Pool) -> Result<State> {
    let updates = query_updates(pool).await?;
    Ok(State::collect(updates.into_iter().rev())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx_database_tester::test(
        pool(variable = "migrated_pool", migrations = "./migrations"),
        pool(
            variable = "empty_db_pool",
            transaction_variable = "empty_db_transaction",
            skip_migrations
        )
    )]
    async fn test_server_start() {
        let migrated_pool_tables = sqlx::query!("SELECT * FROM pg_catalog.pg_tables")
            .fetch_all(&migrated_pool)
            .await
            .unwrap();
        let empty_pool_tables = sqlx::query!("SELECT * FROM pg_catalog.pg_tables")
            .fetch_all(&empty_db_pool)
            .await
            .unwrap();
        println!("Migrated pool tables: \n {:#?}", migrated_pool_tables);
        println!("Empty pool tables: \n {:#?}", empty_pool_tables);
    }

    #[sqlx_database_tester::test(pool(variable = "pool"))]
    async fn test_query_updates() {
        // First check that empty db operates well
        let updates = query_updates(&pool).await.unwrap();
        assert_eq!(updates, vec![]);

        // Add single update
        let htlc_update1 = HtlcUpdate {
            sats: 100,
            rate: 2500,
            channel_id: "aboba".to_owned(),
        };
        insert_update(&pool, UpdateBody::Htlc(htlc_update1.clone()))
            .await
            .unwrap();
        let updates: Vec<UpdateBody> = query_updates(&pool)
            .await
            .unwrap()
            .iter()
            .map(|u| u.body.clone())
            .collect();
        assert_eq!(updates, vec![UpdateBody::Htlc(htlc_update1.clone())]);

        // Add second update
        let htlc_update2 = HtlcUpdate {
            sats: 200,
            rate: 2500,
            channel_id: "aboba".to_owned(),
        };
        insert_update(&pool, UpdateBody::Htlc(htlc_update2.clone()))
            .await
            .unwrap();
        let updates: Vec<UpdateBody> = query_updates(&pool)
            .await
            .unwrap()
            .iter()
            .map(|u| u.body.clone())
            .collect();
        assert_eq!(
            updates,
            vec![
                UpdateBody::Htlc(htlc_update2),
                UpdateBody::Htlc(htlc_update1)
            ]
        );

        // Add snapshot, we shouldn't select older updates after the snapshot
        let snapshot_update = StateSnapshot {
            channels_hedge: hashmap! {
                "aboba".to_owned() => ChannelHedge {
                    sats: 300,
                    rate: 2500,
                }
            },
        };
        insert_update(&pool, UpdateBody::Snapshot(snapshot_update.clone()))
            .await
            .unwrap();
        let updates: Vec<UpdateBody> = query_updates(&pool)
            .await
            .unwrap()
            .iter()
            .map(|u| u.body.clone())
            .collect();
        assert_eq!(updates, vec![UpdateBody::Snapshot(snapshot_update.clone())]);

        // Add update after the snapshot
        let htlc_update3 = HtlcUpdate {
            sats: 500,
            rate: 2500,
            channel_id: "aboba".to_owned(),
        };
        insert_update(&pool, UpdateBody::Htlc(htlc_update3.clone()))
            .await
            .unwrap();
        let updates: Vec<UpdateBody> = query_updates(&pool)
            .await
            .unwrap()
            .iter()
            .map(|u| u.body.clone())
            .collect();
        assert_eq!(
            updates,
            vec![
                UpdateBody::Htlc(htlc_update3),
                UpdateBody::Snapshot(snapshot_update.clone())
            ]
        );
    }

    #[sqlx_database_tester::test(pool(variable = "pool"))]
    async fn test_state_fold() {
        let snapshot_update = StateSnapshot {
            channels_hedge: hashmap! {
                "aboba".to_owned() => ChannelHedge {
                    sats: 300,
                    rate: 2500,
                }
            },
        };
        insert_update(&pool, UpdateBody::Snapshot(snapshot_update.clone()))
            .await
            .unwrap();

        let htlc_update1 = HtlcUpdate {
            sats: 100,
            rate: 2500,
            channel_id: "aboba".to_owned(),
        };
        insert_update(&pool, UpdateBody::Htlc(htlc_update1.clone()))
            .await
            .unwrap();

        let htlc_update2 = HtlcUpdate {
            sats: 500,
            rate: 2500,
            channel_id: "aboba".to_owned(),
        };
        insert_update(&pool, UpdateBody::Htlc(htlc_update2.clone()))
            .await
            .unwrap();
        let state = query_state(&pool).await.unwrap();
        assert_eq!(
            state,
            State {
                last_changed: state.last_changed,
                channels_hedge: hashmap! {
                    "aboba".to_owned() => ChannelHedge {
                        sats: 900,
                        rate: 2500,
                    }
                },
                opened_orders: vec![],
                opened_position: None,
            }
        );
    }
}
