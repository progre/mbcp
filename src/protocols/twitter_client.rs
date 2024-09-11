use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use futures::future::join_all;
use serde_json::{json, Value};
use tracing::info;

use crate::{sources::source, store};

use super::twitter_api::{Api, TweetBody};

pub const ORIGIN: &str = "https://twitter.com";

pub struct Client {
    http_client: Arc<reqwest::Client>,
    api: Api,
}

impl Client {
    #[tracing::instrument(name = "twitter_client::Client::new", skip_all)]
    pub async fn new(
        http_client: Arc<reqwest::Client>,
        api_key: String,
        api_key_secret: String,
        access_token: String,
        access_token_secret: String,
    ) -> Result<Self> {
        let api = Api::new(
            http_client.clone(),
            api_key,
            api_key_secret,
            access_token,
            access_token_secret,
        );
        let json: Value = api.verify_credentials().await?;
        let user_id = json
            .get("id_str")
            .ok_or_else(|| anyhow!("id_str is not found"))?
            .as_str()
            .ok_or_else(|| anyhow!("id_str is not str"))?
            .to_owned();
        info!("logged in as {}", user_id);

        Ok(Self { http_client, api })
    }
}

#[async_trait]
impl super::Client for Client {
    fn to_session(&self) -> Option<String> {
        None
    }

    #[tracing::instrument(name = "twitter_client::Client::fetch_statuses", skip_all)]
    async fn fetch_statuses(&mut self) -> Result<Vec<source::LiveStatus>> {
        todo!()
    }

    #[tracing::instrument(name = "twitter_client::Client::post", skip_all)]
    async fn post(
        &mut self,
        content: &str,
        _facets: &[store::operations::Facet],
        reply_identifier: Option<&str>,
        images: Vec<store::operations::Medium>,
        _external: Option<store::operations::External>,
        _created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let media = if images.is_empty() {
            None
        } else {
            // TODO: alt
            let media_ids = join_all(images.into_iter().map(|image| async {
                let resp = self.http_client.get(image.url).send().await?;
                let res: Value = self.api.upload(resp).await?;
                Ok(res)
            }))
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|res: Value| {
                Ok(res
                    .get("media_id_string")
                    .ok_or_else(|| anyhow!("media_id_string is not found"))?
                    .as_str()
                    .ok_or_else(|| anyhow!("media_id_string is not str"))?
                    .to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
            Some(json!({ "media_ids": media_ids }))
        };

        let body = TweetBody {
            media,
            quote_tweet_id: None,
            reply: reply_identifier.map(
                |reply_identifier| serde_json::json!({ "in_reply_to_tweet_id": reply_identifier }),
            ),
            text: content,
        };

        let json: Value = self.api.create_tweet(body).await?;
        let id = json
            .get("data")
            .ok_or_else(|| anyhow!("data is not found"))?
            .as_object()
            .ok_or_else(|| anyhow!("data is not object"))?
            .get("id")
            .ok_or_else(|| anyhow!("id is not found"))?
            .as_str()
            .ok_or_else(|| anyhow!("id is not str"))?;
        Ok(id.to_owned())
    }

    #[tracing::instrument(name = "twitter_client::Client::repost", skip_all)]
    async fn repost(
        &mut self,
        target_identifier: &str,
        _created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let result = self
            .api
            .create_retweet_1_1::<Value>(target_identifier)
            .await;
        let Err(err) = result else {
            return Ok(target_identifier.into());
        };
        // 1.1 のアクセス違反の場合のみ proxy を使う
        if !err.to_string().contains("403") {
            return Err(err);
        }
        let json: Value = self.api.create_retweet_proxy(target_identifier).await?;
        let id = json
            .get("data")
            .ok_or_else(|| anyhow!("data is not found"))?
            .as_object()
            .ok_or_else(|| anyhow!("data is not object"))?
            .get("id")
            .ok_or_else(|| anyhow!("id is not found"))?
            .as_str()
            .ok_or_else(|| anyhow!("id is not str"))?;
        Ok(id.to_owned())
    }

    #[tracing::instrument(name = "twitter_client::Client::delete_post", skip_all)]
    async fn delete_post(&mut self, identifier: &str) -> Result<()> {
        let _: Value = self.api.delete_tweet(identifier).await?;
        Ok(())
    }

    #[tracing::instrument(name = "twitter_client::Client::delete_repost", skip_all)]
    async fn delete_repost(&mut self, identifier: &str) -> Result<()> {
        let target_identifier = identifier;
        let result = self
            .api
            .delete_retweet_1_1::<Value>(target_identifier)
            .await;
        let Err(err) = result else {
            return Ok(());
        };
        // 1.1 のアクセス違反の場合のみ proxy を使う
        if !err.to_string().contains("403") {
            return Err(err);
        }
        self.api
            .delete_retweet_proxy::<Value>(target_identifier)
            .await?;
        Ok(())
    }
}
