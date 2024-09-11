use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use linkify::LinkFinder;
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use tracing::trace;

use crate::{sources::source, store};

fn get_value<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| {
        anyhow!(
            "{} is not found ({})",
            key,
            serde_json::to_string(&value).unwrap_or_default()
        )
    })
}

fn get_as_string_opt(value: &Value, key: &str) -> Result<Option<String>> {
    Ok(get_value(value, key)?.as_str().map(str::to_owned))
}

fn get_as_string(value: &Value, key: &str) -> Result<String> {
    get_as_string_opt(value, key)?.ok_or_else(|| anyhow!("{} is not str", key))
}

fn get_as_array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    get_value(value, key)?
        .as_array()
        .ok_or_else(|| anyhow!("{} is not array", key))
}

fn create_facets(content: &str) -> Vec<store::operations::Facet> {
    LinkFinder::new()
        .links(content)
        .map(|link| store::operations::Facet::Link {
            byte_slice: link.start() as u32..link.end() as u32,
            uri: link.as_str().to_owned(),
        })
        .collect()
}

pub struct Client {
    http_client: Arc<reqwest::Client>,
    origin: String,
    access_token: String,
    user_id: String,
}

impl Client {
    #[tracing::instrument(name = "misskey_client::Client::new", skip_all)]
    pub async fn new(
        http_client: Arc<reqwest::Client>,
        origin: String,
        access_token: String,
    ) -> Result<Self> {
        let resp = http_client
            .post(format!("{}/api/i", origin))
            .json(&json!({ "i": access_token }))
            .send()
            .await?;
        let json: Value = resp.json().await?;
        let user_id = get_as_string(&json, "id")?;
        Ok(Self {
            http_client,
            origin,
            access_token,
            user_id,
        })
    }
}

#[async_trait]
impl super::Client for Client {
    fn to_session(&self) -> Option<String> {
        None
    }

    #[tracing::instrument(name = "misskey_client::Client::fetch_statuses", skip_all)]
    async fn fetch_statuses(&mut self) -> Result<Vec<source::LiveStatus>> {
        let resp = self
            .http_client
            .post(format!("{}/api/users/notes", self.origin))
            .bearer_auth(self.access_token.to_owned())
            .json(&json!({ "userId": self.user_id, "limit": 100 }))
            .send()
            .await?;
        let json: Value = resp.json().await?;
        let root = json
            .as_array()
            .ok_or_else(|| anyhow!("root is not array"))?;
        Ok(root
            .iter()
            .map(|item| {
                let created_at = DateTime::parse_from_rfc3339(&get_as_string(item, "createdAt")?)?;
                if let Some(renote) = item.get("renote") {
                    let target_src_identifier = get_as_string(renote, "id")?;
                    let target_src_uri = renote
                        .get("uri") // WTF: uri が出力されない
                        .and_then(Value::as_str)
                        .map_or_else(
                            || format!("{}/notes/{}", self.origin, target_src_identifier),
                            str::to_owned,
                        );
                    Ok(source::LiveStatus::Repost(
                        store::operations::CreateRepostOperationStatus {
                            src_identifier: get_as_string(item, "id")?,
                            target_src_identifier,
                            target_src_uri,
                            created_at,
                        },
                    ))
                } else {
                    let identifier = get_as_string(item, "id")?;
                    let uri = item
                        .get("uri") // WTF: uri が出力されない
                        .and_then(Value::as_str)
                        .map_or_else(
                            || format!("{}/notes/{}", self.origin, identifier),
                            str::to_owned,
                        );
                    let content = get_as_string_opt(item, "text")?.unwrap_or_default(); // renote のみの場合は null になる
                    let facets = create_facets(&content);
                    Ok(source::LiveStatus::Post(source::LivePost {
                        identifier,
                        uri,
                        content,
                        facets,
                        reply_src_identifier: get_as_string_opt(item, "replyId")?,
                        media: get_as_array(item, "files")?
                            .iter()
                            .map(|file| {
                                Ok(store::operations::Medium {
                                    url: get_as_string(file, "url")?,
                                    alt: get_as_string_opt(file, "comment")?.unwrap_or_default(),
                                })
                            })
                            .collect::<Result<_>>()?,
                        external: source::LiveExternal::Unknown,
                        created_at,
                    }))
                }
            })
            .collect::<Result<Vec<_>>>()?)
    }

    #[tracing::instrument(name = "misskey_client::Client::post", skip_all)]
    async fn post(
        &mut self,
        content: &str,
        _facets: &[store::operations::Facet],
        reply_identifier: Option<&str>,
        images: Vec<store::operations::Medium>,
        _external: Option<store::operations::External>,
        _created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let mut json = json!({
            "replyId": reply_identifier,
            "text": content,
        });
        if !images.is_empty() {
            let mut media_ids = Vec::new();
            for image in images {
                let resp = self.http_client.get(image.url).send().await?;
                trace!("{:?}", resp);
                let multipart = Form::new().part("file", Part::stream(resp).file_name("file.jpg"));
                let url = format!("{}/api/drive/files/create", self.origin);
                let resp = self
                    .http_client
                    .post(url)
                    .bearer_auth(self.access_token.to_owned())
                    .multipart(multipart)
                    .send()
                    .await?;
                let json: Value = resp.json().await?;
                let media_id = json
                    .get("id")
                    .ok_or_else(|| anyhow!("id is not found"))?
                    .as_str()
                    .ok_or_else(|| anyhow!("id is not str"))?;
                media_ids.push(media_id.to_owned());
            }
            json["mediaIds"] = media_ids.into();
        }
        let resp = self
            .http_client
            .post(format!("{}/api/notes/create", self.origin))
            .bearer_auth(self.access_token.to_owned())
            .json(&json)
            .send()
            .await?;
        let json: Value = resp.json().await?;
        trace!("resp: {}", serde_json::to_string_pretty(&json)?);
        json.as_object()
            .ok_or_else(|| anyhow!("root is not object"))?
            .get("createdNote")
            .ok_or_else(|| anyhow!("createdNote is not found"))?
            .as_object()
            .ok_or_else(|| anyhow!("createdNote is not object"))?
            .get("id")
            .ok_or_else(|| anyhow!("id is not found"))?
            .as_str()
            .ok_or_else(|| anyhow!("id is not str"))
            .map(str::to_owned)
    }

    #[tracing::instrument(name = "misskey_client::Client::repost", skip_all)]
    async fn repost(
        &mut self,
        target_identifier: &str,
        _created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let resp = self
            .http_client
            .post(format!("{}/api/notes/create", self.origin))
            .bearer_auth(self.access_token.to_owned())
            .json(&json!({ "renoteId": target_identifier }))
            .send()
            .await?;
        let json: Value = resp.json().await?;
        trace!("resp: {}", serde_json::to_string_pretty(&json)?);
        json.as_object()
            .ok_or_else(|| anyhow!("root is not object"))?
            .get("createdNote")
            .ok_or_else(|| anyhow!("createdNote is not found"))?
            .as_object()
            .ok_or_else(|| anyhow!("createdNote is not object"))?
            .get("renoteId")
            .ok_or_else(|| anyhow!("renoteId is not found"))?
            .as_str()
            .ok_or_else(|| anyhow!("renoteId is not str"))
            .map(str::to_owned)
    }

    #[tracing::instrument(name = "misskey_client::Client::delete_post", skip_all)]
    async fn delete_post(&mut self, identifier: &str) -> Result<()> {
        let resp = self
            .http_client
            .post(format!("{}/api/notes/delete", self.origin))
            .bearer_auth(self.access_token.to_owned())
            .json(&json!({ "noteId": identifier }))
            .send()
            .await?;
        resp.error_for_status().map(|_| ()).map_err(|e| e.into())
    }

    #[tracing::instrument(name = "misskey_client::Client::delete_repost", skip_all)]
    async fn delete_repost(&mut self, identifier: &str) -> Result<()> {
        let resp = self
            .http_client
            .post(format!("{}/api/notes/unrenote", self.origin))
            .bearer_auth(self.access_token.to_owned())
            .json(&json!({ "noteId": identifier }))
            .send()
            .await?;
        resp.error_for_status().map(|_| ()).map_err(|e| e.into())
    }
}
