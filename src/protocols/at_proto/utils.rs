use anyhow::{anyhow, Result};
use atrium_api::app::bsky::feed::post::ReplyRef;
use chrono::{DateTime, FixedOffset};
use regex::Regex;
use reqwest::header::CONTENT_TYPE;
use serde_json::json;

use crate::store::{self, operations::Facet::Link};

use super::{
    repo::{Embed, External, Image, Record},
    Api, Session,
};

pub struct AtriumClient<'a> {
    http_client: &'a reqwest::Client,
    session: &'a Option<Session>,
}

impl<'a> AtriumClient<'a> {
    pub fn new(http_client: &'a reqwest::Client, session: &'a Option<Session>) -> Self {
        Self {
            http_client,
            session,
        }
    }
}

#[async_trait::async_trait]
impl atrium_api::xrpc::HttpClient for AtriumClient<'_> {
    async fn send(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, Box<dyn std::error::Error>> {
        let resp = self.http_client.execute(req.try_into()?).await?;
        let mut builder = http::Response::builder().status(resp.status());
        for (k, v) in resp.headers() {
            builder = builder.header(k, v);
        }
        builder
            .body(resp.bytes().await?.to_vec())
            .map_err(Into::into)
    }
}

#[async_trait::async_trait]
impl atrium_api::xrpc::XrpcClient for AtriumClient<'_> {
    fn host(&self) -> &str {
        "https://bsky.social"
    }
    fn auth(&self, is_refresh: bool) -> Option<&str> {
        self.session.as_ref().map(|session| {
            if is_refresh {
                session.refresh_jwt.as_str()
            } else {
                session.access_jwt.as_str()
            }
        })
    }
}

atrium_api::impl_traits!(AtriumClient<'_>);

pub fn to_record<'a>(
    text: &'a str,
    facets: &'a [store::operations::Facet],
    reply: Option<app::bsky::feed::post::ReplyRef>,
    embed: Option<Embed>,
    created_at: &'a DateTime<FixedOffset>,
) -> Record<'a> {
    Record {
        text,
        facets: facets
            .iter()
            .map(|facet| match facet {
                // NOTE: 実装予定なし
                // Mention {
                //     byte_slice,
                //     src_identifier,
                // } => {
                //     json!({
                //         "index": {
                //             "byteStart": byte_slice.start,
                //             "byteEnd": byte_slice.end
                //         },
                //         "features": [{
                //             "$type": "app.bsky.richtext.facet#mention",
                //             "did": "TODO",
                //         }]
                //     })
                // }
                Link { byte_slice, uri } => json!({
                    "index": {
                        "byteStart": byte_slice.start,
                        "byteEnd": byte_slice.end
                    },
                    "features": [{
                        "$type": "app.bsky.richtext.facet#link",
                        "uri": uri,
                    }]
                }),
            })
            .collect::<Vec<_>>(),
        reply,
        embed: embed.map(|embed| match embed {
            Embed::External(external) => json!({
                "$type": "app.bsky.embed.external",
                "external": external,
            }),
            Embed::Images(images) => json!({
                "$type": "app.bsky.embed.images",
                "images": images,
            }),
        }),
        created_at,
    }
}

pub fn uri_to_post_rkey(uri: &str) -> Result<String> {
    Ok(Regex::new(r"at://did:plc:.+?/app.bsky.feed.post/(.+)")
        .unwrap()
        .captures(uri)
        .ok_or_else(|| anyhow!("invalid uri format"))?[1]
        .to_owned())
}

pub fn uri_to_repost_rkey(uri: &str) -> Result<String> {
    Ok(Regex::new(r"at://did:plc:.+?/app.bsky.feed.repost/(.+)")
        .unwrap()
        .captures(uri)
        .ok_or_else(|| anyhow!("invalid uri format"))?[1]
        .to_owned())
}

pub async fn to_embed(
    api: &Api,
    http_client: &reqwest::Client,
    session: &Session,
    images: Vec<store::operations::Medium>,
    external: Option<store::operations::External>,
) -> Result<Option<Embed>> {
    if !images.is_empty() {
        let mut array = Vec::new();
        for image in images {
            let resp = http_client.get(&image.url).send().await?;
            let content_type = resp
                .headers()
                .get(CONTENT_TYPE)
                .ok_or_else(|| anyhow!("no content-type"))?
                .to_str()?
                .to_owned();

            let mut res = api
                .repo
                .upload_blob(http_client, session, content_type, resp)
                .await?;
            let alt = image.alt;
            let image = res
                .get_mut("blob")
                .ok_or_else(|| anyhow!("blob not found"))?
                .take();
            array.push(Image { image, alt });
        }
        return Ok(Some(Embed::Images(array)));
    }
    if let Some(external) = external {
        if let Some(thumb_url) = &external.thumb_url {
            let resp = http_client.get(thumb_url).send().await?;
            let content_type = resp
                .headers()
                .get(CONTENT_TYPE)
                .ok_or_else(|| anyhow!("no content-type"))?
                .to_str()?
                .to_owned();

            let mut res = api
                .repo
                .upload_blob(http_client, session, content_type, resp)
                .await?;
            let thumb = res
                .get_mut("blob")
                .ok_or_else(|| anyhow!("blob not found"))?
                .take();
            return Ok(Some(Embed::External(External {
                uri: external.uri,
                title: external.title,
                description: external.description,
                thumb,
            })));
        }
    }
    Ok(None)
}

pub async fn find_reply_root(
    api: &Api,
    http_client: &reqwest::Client,
    session: &Session,
    rkey: &str,
) -> Result<Option<com::atproto::repo::strong_ref::Main>> {
    let record = api.repo.get_record(http_client, session, rkey).await?;
    let atrium_api::records::Record::AppBskyFeedPost(record) = record.value else {
        unreachable!();
    };
    let Some(reply) = record.reply else {
        return Ok(None);
    };

    Ok(Some(reply.root))
}

pub async fn to_reply<'a>(
    api: &Api,
    http_client: &reqwest::Client,
    session: &Session,
    reply_identifier: Option<&str>,
) -> Result<Option<ReplyRef>> {
    let Some(reply_identifier) = reply_identifier else {
        return Ok(None);
    };
    let parent: com::atproto::repo::strong_ref::Main = serde_json::from_str(reply_identifier)?;
    let root = find_reply_root(api, http_client, session, &uri_to_post_rkey(&parent.uri)?)
        .await?
        .unwrap_or_else(|| parent.clone());
    Ok(Some(app::bsky::feed::post::ReplyRef { parent, root }))
}
