use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tracing::error;

use self::repo::Repo;

pub mod from_atrium;
pub mod repo;
pub mod utils;

pub struct Api {
    pub repo: Repo,
}

impl Api {
    pub fn new(origin: String) -> Self {
        Self {
            repo: Repo::new(origin.clone()),
        }
    }
}

async fn query<T: DeserializeOwned, U: Serialize + ?Sized>(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    lexicon_id: &str,
    query_params: &U,
) -> Result<T> {
    let resp = client
        .get(format!("{}/xrpc/{}", origin, lexicon_id))
        .query(query_params)
        .bearer_auth(token)
        .send()
        .await?;
    if let Err(err) = resp.error_for_status_ref() {
        let json: Value = resp.json().await?;
        error!(
            "url={:?}, status-code={:?}, body={}",
            err.url().map(ToString::to_string),
            err.status(),
            json
        );
        return Err(err.into());
    }
    Ok(resp.json().await?)
}

async fn procedure<T: DeserializeOwned>(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    lexicon_id: &str,
    properties: &Value,
) -> Result<T> {
    let resp = client
        .post(format!("{}/xrpc/{}", origin, lexicon_id))
        .bearer_auth(token)
        .json(properties)
        .send()
        .await?;
    if let Err(err) = resp.error_for_status_ref() {
        let json: Value = resp.json().await?;
        error!(
            "url={:?}, status-code={:?}, body={}",
            err.url().map(ToString::to_string),
            err.status(),
            json
        );
        return Err(err.into());
    }
    Ok(resp.json().await?)
}
