use anyhow::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tracing::error;

use self::{repo::Repo, server::Server};

pub mod repo;
pub mod server;
pub mod utils;

pub struct Api {
    pub origin: String,
    pub repo: Repo,
    pub server: Server,
}

impl Api {
    pub fn new(origin: String) -> Self {
        Self {
            origin: origin.clone(),
            repo: Repo::new(origin.clone()),
            server: Server::new(origin),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub did: String,
    handle: String,
    email: String,
    pub access_jwt: String,
    refresh_jwt: String,
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
