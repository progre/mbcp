use std::{
    convert::Into,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use tracing::trace;

use crate::{
    app::AccountKey,
    config,
    protocols::{create_client, Client},
    store::{
        self,
        operations::Operation::{CreatePost, CreateRepost, DeletePost, DeleteRepost, UpdatePost},
        user::SourceStatus::{Post, Repost},
    },
};

use super::{merge_operations::merge_operations, operation_factory::create_operations};

#[derive(Clone, Debug)]
pub enum LiveExternal {
    Some(store::operations::External),
    None,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct LivePost {
    pub identifier: String,
    pub uri: String,
    pub content: String,
    pub facets: Vec<store::operations::Facet>,
    pub reply_src_identifier: Option<String>,
    pub media: Vec<store::operations::Medium>,
    pub external: LiveExternal,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Clone, Debug)]
pub enum LiveStatus {
    Post(LivePost),
    Repost(store::operations::CreateRepostOperationStatus),
}

impl LiveStatus {
    pub fn created_at(&self) -> &DateTime<FixedOffset> {
        match self {
            LiveStatus::Post(LivePost { created_at, .. })
            | LiveStatus::Repost(store::operations::CreateRepostOperationStatus {
                created_at,
                ..
            }) => created_at,
        }
    }
}

#[derive(Debug)]
pub enum Operation {
    CreatePost(store::operations::CreatePostOperationStatus),
    CreateRepost(store::operations::CreateRepostOperationStatus),
    UpdatePost(store::operations::UpdatePostOperationStatus),
    DeletePost(store::operations::DeletePostOperationStatus),
    DeleteRepost(store::operations::DeleteRepostOperationStatus),
}

impl Operation {
    pub fn to_store(
        &self,
        account_pair: store::operations::AccountPair,
    ) -> store::operations::Operation {
        match self {
            Operation::CreatePost(status) => CreatePost(store::operations::CreatePostOperation {
                account_pair,
                status: status.clone(),
            }),
            Operation::CreateRepost(status) => {
                CreateRepost(store::operations::CreateRepostOperation {
                    account_pair,
                    status: status.clone(),
                })
            }
            Operation::UpdatePost(status) => UpdatePost(store::operations::UpdatePostOperation {
                account_pair,
                status: status.clone(),
            }),
            Operation::DeletePost(status) => DeletePost(store::operations::DeletePostOperation {
                account_pair,
                status: status.clone(),
            }),
            Operation::DeleteRepost(status) => {
                DeleteRepost(store::operations::DeleteRepostOperation {
                    account_pair,
                    status: status.clone(),
                })
            }
        }
    }
}

async fn fetch_statuses(
    src_client: &mut dyn Client,
    http_client: &reqwest::Client,
    src_statuses: &[store::user::SourceStatus],
) -> Result<(Vec<store::user::SourceStatus>, Vec<Operation>)> {
    let live_statuses = src_client.fetch_statuses().await?;

    let operations = create_operations(http_client, &live_statuses, src_statuses).await?;
    let statuses: Vec<_> = live_statuses.into_iter().map(Into::into).collect();
    Ok((statuses, operations))
}

fn has_users_operations(operations: &[store::operations::Operation], src_key: &AccountKey) -> bool {
    operations
        .iter()
        .any(|operation| &operation.account_pair().to_src_key() == src_key)
}

pub async fn get(
    http_client: &Arc<reqwest::Client>,
    config_user: &config::User,
    store: &Mutex<&mut store::Store>,
) -> Result<()> {
    let session = store
        .lock()
        .unwrap()
        .get_or_create_user_mut(&config_user.src.to_account_key())
        .src
        .session
        .clone();

    let mut src_client = create_client(http_client.clone(), &config_user.src, session).await?;
    {
        let mut store = store.lock().unwrap();
        store
            .get_or_create_user_mut(&config_user.src.to_account_key())
            .src
            .session = src_client.to_session();
    }

    let src_account_key = config_user.src.to_account_key();
    let (has_users_operations, src_statuses) = {
        let mut store = store.lock().unwrap();
        let has_users_operations = has_users_operations(&store.operations, &src_account_key);
        let stored_user = store.get_or_create_user_mut(&src_account_key);
        (has_users_operations, &stored_user.src.statuses.clone())
    };

    let (statuses, operations) =
        fetch_statuses(src_client.as_mut(), http_client.as_ref(), src_statuses).await?;

    {
        let mut store = store.lock().unwrap();
        let stored_user = store.get_or_create_user_mut(&src_account_key);
        stored_user.src.statuses = statuses;
    }
    trace!("new operations: {:?}", operations);
    if operations.is_empty() && !has_users_operations {
        return Ok(());
    }

    let dst_account_keys: Vec<AccountKey> = config_user
        .dsts
        .iter()
        .map(|dst| dst.to_account_key())
        .collect();

    if !operations.is_empty() {
        let mut store = store.lock().unwrap();
        merge_operations(&mut store, &dst_account_keys, &src_account_key, &operations);
    }
    Ok(())
}

/**
 * src が参照している post の identifier を全て返す
 */
fn necessary_post_src_identifiers(users: &[store::user::User]) -> Vec<String> {
    users
        .iter()
        .flat_map(|user| user.src.statuses.iter())
        .map(|src_status| match src_status {
            Post(post) => post.identifier.clone(),
            Repost(repost) => repost.target_identifier.clone(),
        })
        .collect()
}

fn necessary_repost_src_identifiers(users: &[store::user::User]) -> Vec<String> {
    users
        .iter()
        .flat_map(|user| user.src.statuses.iter())
        .filter_map(|src_status| match src_status {
            Post(_) => None,
            Repost(repost) => Some(repost.identifier.clone()),
        })
        .collect()
}

pub async fn retain_all_dst_statuses(store: &mut store::Store) -> Result<()> {
    let necessary_post_src_identifiers = necessary_post_src_identifiers(&store.users);
    let necessary_repost_src_identifiers = necessary_repost_src_identifiers(&store.users);

    store
        .users
        .iter_mut()
        .flat_map(|user| user.dsts.iter_mut())
        .for_each(|dst| {
            dst.statuses.retain(|status| match status {
                store::user::DestinationStatus::Post(post) => {
                    necessary_post_src_identifiers.contains(&post.src_identifier)
                }
                store::user::DestinationStatus::Repost(repost) => {
                    necessary_repost_src_identifiers.contains(&repost.src_identifier)
                }
            });
        });
    Ok(())
}
