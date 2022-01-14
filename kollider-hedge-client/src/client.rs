use kollider_hedge_domain::state::*;
use kollider_hedge_domain::api::*;
use serde::{Serialize, Deserialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Reqwesting server error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("JSON encoding/decoding error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Alias for a `Result` with the error type `self::Error`.
pub type Result<T> = std::result::Result<T, Error>;

pub struct HedgeClient {
    pub client: reqwest::Client,
    pub server: String,
}

impl HedgeClient {
    pub fn new(url: &str) -> Self {
        HedgeClient {
            client: reqwest::Client::new(),
            server: url.to_owned(),
        }
    }

    pub async fn hedge_htlc(&self, info: HtlcInfo) -> Result<()> {
        let path = "/hedge_htlc";
        let endpoint = format!("{}{}", self.server, path);
        self.client.post(endpoint).json(&info).build()?;
        // let response = self.client.execute(request).await?.text().await?;
        // println!("Response: {}", response);
        // Ok(serde_json::from_str(&response)?)
        Ok(())
    }
}
