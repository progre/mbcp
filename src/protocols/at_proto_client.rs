use std::{str::FromStr, sync::Arc};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use atrium_api::{
    agent::{store::MemorySessionStore, AtpAgent},
    app, com,
    record::KnownRecord,
    types::{
        string::{Datetime, Nsid},
        LimitedNonZeroU8, Object, TryIntoUnknown,
    },
};
use atrium_xrpc_client::reqwest::ReqwestClient;
use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use crate::{sources::source, store};

use super::at_proto::{
    utils::{to_embed, to_record, to_reply, uri_to_post_rkey, uri_to_repost_rkey},
    Api,
};

pub struct Client {
    agent: AtpAgent<MemorySessionStore, ReqwestClient>,
    api: Api,
    http_client: Arc<reqwest::Client>,
    session: Option<com::atproto::server::create_session::Output>,
    pub identifier: String,
    password: String,
}

impl Client {
    #[tracing::instrument(name = "at_proto_client::Client::new", skip_all)]
    pub async fn new(
        origin: String,
        http_client: Arc<reqwest::Client>,
        identifier: String,
        password: String,
    ) -> Result<Self> {
        let agent = AtpAgent::new(
            ReqwestClient::new("https://bsky.social"),
            MemorySessionStore::default(),
        );
        agent.login(&identifier, &password).await?;
        Ok(Self {
            agent,
            api: Api::new(origin),
            http_client,
            session: None,
            identifier,
            password,
        })
    }

    #[tracing::instrument(name = "at_proto_client::Client::init_session", skip_all)]
    async fn init_session(&mut self) -> Result<()> {
        let input = Object::from(com::atproto::server::create_session::InputData {
            identifier: self.identifier.clone(),
            password: self.password.clone(),
            auth_factor_token: None,
        });
        let session = self
            .agent
            .api
            .com
            .atproto
            .server
            .create_session(input)
            .await
            .map_err(|err| anyhow::anyhow!("{:?}", err))?;
        self.session = Some(session);
        Ok(())
    }
}

#[async_trait]
impl super::Client for Client {
    fn origin(&self) -> &str {
        &self.api.origin
    }

    fn identifier(&self) -> &str {
        &self.identifier
    }

    #[tracing::instrument(name = "at_proto_client::Client::fetch_statuses", skip_all)]
    async fn fetch_statuses(&mut self) -> Result<Vec<source::LiveStatus>> {
        let session = match &self.session {
            Some(some) => some,
            None => {
                self.init_session().await?;
                self.session.as_ref().unwrap()
            }
        };

        let params = Object::from(app::bsky::feed::get_author_feed::ParametersData {
            actor: session.did.clone().into(),
            cursor: None,
            filter: None,
            limit: Some(LimitedNonZeroU8::try_from(50).unwrap()),
        });
        let output = self
            .agent
            .api
            .app
            .bsky
            .feed
            .get_author_feed(params)
            .await
            .map_err(|err| anyhow::anyhow!("{:?}", err))?;
        output.data.feed.into_iter().map(|x| x.try_into()).collect()
    }

    #[tracing::instrument(name = "at_proto_client::Client::post", skip_all)]
    async fn post(
        &mut self,
        content: &str,
        facets: &[store::operations::Facet],
        reply_identifier: Option<&str>,
        images: Vec<store::operations::Medium>,
        external: Option<store::operations::External>,
        created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let session = match &self.session {
            Some(some) => some,
            None => {
                self.init_session().await?;
                self.session.as_ref().unwrap()
            }
        };
        let reply = to_reply(&self.api, &self.http_client, session, reply_identifier).await?;
        let embed = to_embed(&self.api, &self.http_client, session, images, external).await?;
        let record = to_record(content, facets, reply, embed, created_at);

        let output = self
            .api
            .repo
            .create_record(&self.http_client, session, record)
            .await?;
        Ok(serde_json::to_string(&output)?)
    }

    #[tracing::instrument(name = "at_proto_client::Client::repost", skip_all)]
    async fn repost(
        &mut self,
        target_identifier: &str,
        created_at: &DateTime<FixedOffset>,
    ) -> Result<String> {
        let session = match &self.session {
            Some(some) => some,
            None => {
                self.init_session().await?;
                self.session.as_ref().unwrap()
            }
        };

        let identifier: com::atproto::repo::create_record::Output =
            serde_json::from_str(target_identifier)?;
        let record = KnownRecord::AppBskyFeedRepost(Box::new(Object::from(
            app::bsky::feed::repost::RecordData {
                created_at: Datetime::new(created_at.to_owned()),
                subject: Object::from(com::atproto::repo::strong_ref::MainData {
                    cid: identifier.data.cid,
                    uri: identifier.data.uri,
                }),
            },
        )));
        let res = self
            .agent
            .api
            .com
            .atproto
            .repo
            .create_record(Object::from(com::atproto::repo::create_record::InputData {
                collection: Nsid::from_str("app.bsky.feed.repost").unwrap(),
                record: record.try_into_unknown()?,
                repo: session.did.clone().into(),
                rkey: None,
                swap_commit: None,
                validate: None,
            }))
            .await
            .map_err(|err| anyhow::anyhow!("{:?}", err))?;
        Ok(serde_json::to_string(&res)?)
    }

    #[tracing::instrument(name = "at_proto_client::Client::delete_post", skip_all)]
    async fn delete_post(&mut self, identifier: &str) -> Result<()> {
        let json: Value = serde_json::from_str(identifier)?;
        let uri = json
            .get("uri")
            .ok_or_else(|| anyhow!("uri not found ({})", identifier))?
            .as_str()
            .ok_or_else(|| anyhow!("uri is not string"))?;
        let rkey = uri_to_post_rkey(uri)?;

        let session = match &self.session {
            Some(some) => some,
            None => {
                self.init_session().await?;
                self.session.as_ref().unwrap()
            }
        };

        self.api
            .repo
            .delete_record(&self.http_client, session, &rkey)
            .await?;
        Ok(())
    }

    #[tracing::instrument(name = "at_proto_client::Client::delete_repost", skip_all)]
    async fn delete_repost(&mut self, identifier: &str) -> Result<()> {
        let output: com::atproto::repo::put_record::Output = serde_json::from_str(identifier)?;
        let rkey = uri_to_repost_rkey(&output.uri)?;

        let session = match &self.session {
            Some(some) => some,
            None => {
                self.init_session().await?;
                self.session.as_ref().unwrap()
            }
        };

        let input = Object::from(com::atproto::repo::delete_record::InputData {
            collection: Nsid::from_str("app.bsky.feed.repost").unwrap(),
            repo: session.did.clone().into(),
            rkey,
            swap_commit: None,
            swap_record: None,
        });
        self.agent
            .api
            .com
            .atproto
            .repo
            .delete_record(input)
            .await
            .map_err(|err| anyhow::anyhow!("{:?}", err))?;

        Ok(())
    }
}
