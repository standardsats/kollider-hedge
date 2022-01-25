use kollider_hedge_domain::api::*;
use kollider_hedge_domain::state::*;
use log::*;
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
        let path = "/hedge/htlc";
        let endpoint = format!("{}{}", self.server, path);
        let request = self.client.post(endpoint).json(&info).build()?;
        self.client
            .execute(request)
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(())
    }

    pub async fn query_state(&self) -> Result<State> {
        let path = "/state";
        let endpoint = format!("{}{}", self.server, path);
        let request = self.client.get(endpoint).build()?;
        let response = self
            .client
            .execute(request)
            .await?
            .error_for_status()?
            .text()
            .await?;
        debug!("Response: {}", response);
        Ok(serde_json::from_str(&response)?)
    }

    pub async fn query_stats(&self) -> Result<Stats> {
        let path = "/stats";
        let endpoint = format!("{}{}", self.server, path);
        let request = self.client.get(endpoint).build()?;
        let response = self
            .client
            .execute(request)
            .await?
            .error_for_status()?
            .text()
            .await?;
        debug!("Response: {}", response);
        Ok(serde_json::from_str(&response)?)
    }
}
