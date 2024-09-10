use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use futures::future::join_all;
use http::header::ACCEPT;
use megalodon::{
    megalodon::{GetAccountStatusesInputOptions, PostStatusInputOptions, PostStatusOutput},
    Megalodon,
};
use reqwest::{header::HeaderMap, multipart::Part, Body};
use tracing::{debug, event_enabled, trace, Level};

use crate::{sources::source, store};

fn trace_header(header: &HeaderMap) {
    if !event_enabled!(Level::TRACE) {
        return;
    }
    header
        .iter()
        .filter(|(key, _)| {
            [
                "date",
                "x-ratelimit-limit",
                "x-ratelimit-remaining",
                "x-ratelimit-reset",
            ]
            .contains(&key.as_str())
        })
        .for_each(|(key, value)| {
            trace!("{}: {}", key, value.to_str().unwrap_or_default());
        });
}

async fn upload_media(
    origin: &str,
    access_token: &str,
    src_url: &str,
) -> Result<megalodon::response::Response<megalodon::entities::Attachment>> {
    let resp = reqwest::get(src_url).await?;

    let body = Body::from(resp);
    let part = Part::stream(body).file_name("_");
    let form = reqwest::multipart::Form::new().part("file", part);
    let resp = reqwest::Client::new()
        .post(format!("{}{}", origin, "/api/v2/media"))
        .bearer_auth(access_token)
        .multipart(form)
        .header(ACCEPT.as_str(), "application/json")
        .send()
        .await?;
    let status_code = resp.status().as_u16();
    let status_text = resp.status().to_string();
    let headers = resp.headers().to_owned();
    tracing::trace!("{} {} {:?}", status_code, status_text, headers);

    let res = megalodon::response::Response::<megalodon::entities::Attachment>::new(
        resp.json().await?,
        status_code,
        status_text,
        headers,
    );
    Ok(res)
}

async fn upload_media_list(
    origin: &str,
    access_token: &str,
    images: &[store::operations::Medium],
) -> Result<Vec<String>> {
    let upload_media_futures = images
        .iter()
        .map(|image| upload_media(origin, access_token, &image.url));
    Ok(join_all(upload_media_futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|resp| resp.json().id)
        .collect())
}

fn to_megalodon_post_status_input_options(
    media_ids: Vec<String>,
    reply_identifier: Option<&str>,
) -> PostStatusInputOptions {
    PostStatusInputOptions {
        media_ids: if media_ids.is_empty() {
            None
        } else {
            Some(media_ids)
        },
        poll: None,
        in_reply_to_id: reply_identifier.map(|x| x.to_owned()),
        sensitive: None,
        spoiler_text: None,
        visibility: None,
        scheduled_at: None,
        language: None,
        quote_id: None,
    }
}

pub struct Client {
    origin: String,
    access_token: String,
    megalodon: Box<dyn Megalodon + Send + Sync>,
    account_id: String,
}

impl Client {
    #[tracing::instrument(name = "megalodon_client::Client::new", skip_all)]
    pub async fn new_mastodon(origin: String, access_token: String) -> Result<Self> {
        let megalodon = megalodon::generator(
            megalodon::SNS::Mastodon,
            origin.clone(),
            Some(access_token.clone()),
            None,
        );
        let resp = megalodon.verify_account_credentials().await?;
        trace_header(&resp.header);
        let account_id = resp.json().id;

        Ok(Self {
            origin,
            access_token,
            megalodon,
            account_id,
        })
    }
}

#[async_trait]
impl super::Client for Client {
    #[tracing::instrument(name = "megalodon_client::Client::fetch_statuses", skip_all)]
    async fn fetch_statuses(&mut self) -> Result<Vec<source::LiveStatus>> {
        let resp = self
            .megalodon
            .get_account_statuses(
                self.account_id.clone(),
                Some(&GetAccountStatusesInputOptions {
                    limit: Some(40),
                    // exclude_replies: Some(true), // TODO: include self replies
                    ..Default::default()
                }),
            )
            .await?;
        trace_header(&resp.header);
        let statuses: Vec<_> = resp
            .json()
            .into_iter()
            .map(|status| status.into())
            .collect();

        Ok(statuses)
    }

    #[tracing::instrument(name = "megalodon_client::Client::post", skip_all)]
    async fn post(
        &mut self,
        content: &str,
        _facets: &[store::operations::Facet],
        reply_identifier: Option<&str>,
        images: Vec<store::operations::Medium>,
        _external: Option<store::operations::External>,
        _created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let media_ids = upload_media_list(&self.origin, &self.access_token, &images).await?;
        if let PostStatusOutput::Status(status) = self
            .megalodon
            .post_status(
                content.to_owned(),
                Some(&to_megalodon_post_status_input_options(
                    media_ids,
                    reply_identifier,
                )),
            )
            .await?
            .json()
        {
            Ok(status.id)
        } else {
            unreachable!()
        }
    }

    #[tracing::instrument(name = "megalodon_client::Client::repost", skip_all)]
    async fn repost(
        &mut self,
        target_identifier: &str,
        _created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let res = self
            .megalodon
            .reblog_status(target_identifier.to_owned())
            .await?;
        Ok(res.json().id)
    }

    #[tracing::instrument(name = "megalodon_client::Client::delete_post", skip_all)]
    async fn delete_post(&mut self, identifier: &str) -> Result<()> {
        let result = self.megalodon.delete_status(identifier.to_owned()).await;
        debug!("megalodon delete_post: {:?}", result);
        const IGNORE_ERROR_MSG: &str =
            "error decoding response body: invalid type: map, expected unit at line 1 column 0";
        match result {
            Ok(_) => Ok(()),
            // WTF
            Err(megalodon::error::Error::RequestError(err))
                if err.is_decode()
                    && err.status().is_none()
                    && err.to_string() == IGNORE_ERROR_MSG =>
            {
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }

    #[tracing::instrument(name = "megalodon_client::Client::delete_repost", skip_all)]
    async fn delete_repost(&mut self, identifier: &str) -> Result<()> {
        let result = self.megalodon.delete_status(identifier.to_owned()).await;
        debug!("megalodon delete_repost: {:?}", result);
        const IGNORE_ERROR_MSG: &str =
            "error decoding response body: invalid type: map, expected unit at line 1 column 0";
        match result {
            Ok(_) => Ok(()),
            // WTF
            Err(megalodon::error::Error::RequestError(err))
                if err.is_decode()
                    && err.status().is_none()
                    && err.to_string() == IGNORE_ERROR_MSG =>
            {
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }
}
