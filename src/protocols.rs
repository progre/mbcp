mod at_proto;
pub mod at_proto_client;
mod from_megalodon;
pub mod megalodon_client;
mod misskey_client;
mod twitter_api;
pub mod twitter_client;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};

use crate::{config, sources::source, store};

#[async_trait]
pub trait Client: Send + Sync {
    fn to_session(&self) -> Option<String>;

    async fn fetch_statuses(&mut self) -> Result<Vec<source::LiveStatus>>;

    async fn post(
        &mut self,
        content: &str,
        facets: &[store::operations::Facet],
        reply_identifier: Option<&str>,
        images: Vec<store::operations::Medium>,
        external: Option<store::operations::External>,
        created_at: &DateTime<FixedOffset>,
    ) -> Result<String>;

    async fn repost(
        &mut self,
        target_identifier: &str,
        created_at: &DateTime<FixedOffset>,
    ) -> Result<String>;

    async fn delete_post(&mut self, identifier: &str) -> Result<()>;

    async fn delete_repost(&mut self, identifier: &str) -> Result<()>;
}

pub async fn create_client(
    http_client: Arc<reqwest::Client>,
    account: &config::Account,
    initial_session: Option<String>,
) -> Result<Box<dyn Client>> {
    match account {
        config::Account::AtProtocol {
            origin,
            identifier,
            password,
        } => Ok(Box::new(
            at_proto_client::Client::new(
                origin.into(),
                http_client,
                identifier.into(),
                password.into(),
                initial_session,
            )
            .await?,
        )),
        config::Account::Mastodon {
            origin,
            access_token,
        } => Ok(Box::new(
            megalodon_client::Client::new_mastodon(origin.clone(), access_token.clone()).await?,
        )),
        config::Account::Misskey {
            origin,
            access_token,
        } => Ok(Box::new(
            misskey_client::Client::new(http_client, origin.clone(), access_token.clone()).await?,
        )),
        config::Account::Twitter {
            api_key,
            api_key_secret,
            access_token,
            access_token_secret,
        } => Ok(Box::new(
            twitter_client::Client::new(
                http_client,
                api_key.clone(),
                api_key_secret.clone(),
                access_token.clone(),
                access_token_secret.clone(),
            )
            .await?,
        )),
    }
}
