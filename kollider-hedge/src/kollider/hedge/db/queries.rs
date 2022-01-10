use futures::StreamExt;
use super::consts::Pool;
use super::scheme::{StateUpdate, UpdateBodyError, UpdateTag};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Failed to decode body by tag: {0}")]
    UpdateBody(#[from] UpdateBodyError),
}

/// Alias for a `Result` with the error type `self::Error`.
pub type Result<T> = std::result::Result<T, Error>;

/// Query all history of updates until we hit a snapshot or the begining of time
pub async fn query_updates(pool: Pool) -> Result<Vec<StateUpdate>> {
    let mut conn = pool.acquire().await?;
    let res = sqlx::query!("select * from updates order by created desc").fetch(&mut conn).fuse();
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
