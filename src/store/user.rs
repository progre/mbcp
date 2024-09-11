use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::{app::AccountKey, sources::source, utils::format_rfc3339};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePost {
    pub identifier: String,
    pub content: String,
    #[serde(with = "format_rfc3339")]
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRepost {
    pub identifier: String,
    pub target_identifier: String,
    #[serde(with = "format_rfc3339")]
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum SourceStatus {
    Post(SourcePost),
    Repost(SourceRepost),
}

impl SourceStatus {
    pub fn created_at(&self) -> &DateTime<FixedOffset> {
        match self {
            SourceStatus::Post(SourcePost { created_at, .. })
            | SourceStatus::Repost(SourceRepost { created_at, .. }) => created_at,
        }
    }
}

impl From<super::operations::CreatePostOperationStatus> for SourceStatus {
    fn from(full: super::operations::CreatePostOperationStatus) -> Self {
        SourceStatus::Post(SourcePost {
            identifier: full.src_identifier,
            content: full.content,
            created_at: full.created_at,
        })
    }
}

impl From<source::LiveStatus> for SourceStatus {
    fn from(live: source::LiveStatus) -> Self {
        match live {
            source::LiveStatus::Post(post) => SourceStatus::Post(SourcePost {
                identifier: post.identifier,
                content: post.content,
                created_at: post.created_at,
            }),
            source::LiveStatus::Repost(repost) => SourceStatus::Repost(SourceRepost {
                identifier: repost.src_identifier,
                target_identifier: repost.target_src_identifier,
                created_at: repost.created_at,
            }),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub origin: String,
    pub identifier: String,
    pub session: Option<String>,
    pub statuses: Vec<SourceStatus>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationPost {
    pub identifier: String,
    pub src_identifier: String,
    pub src_uri: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationRepost {
    pub identifier: String,
    pub src_identifier: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum DestinationStatus {
    Post(DestinationPost),
    Repost(DestinationRepost),
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub origin: String,
    pub identifier: String,
    pub session: Option<String>,
    pub statuses: Vec<DestinationStatus>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub src: Source,
    pub dsts: Vec<Destination>,
}

impl User {
    pub fn get_or_create_dst_mut<'a>(
        &'a mut self,
        account_key: &AccountKey,
    ) -> &'a mut Destination {
        let idx = self.dsts.iter().position(|dst| {
            dst.origin == account_key.origin && dst.identifier == account_key.identifier
        });
        if let Some(idx) = idx {
            return &mut self.dsts[idx];
        }
        self.dsts.push(Destination {
            origin: account_key.origin.clone(),
            identifier: account_key.identifier.clone(),
            session: None,
            statuses: Vec::default(),
        });
        self.dsts.last_mut().unwrap()
    }
}
