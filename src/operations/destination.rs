use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace, warn};

use crate::{
    config::Account,
    protocols::create_client,
    store::{
        self,
        operations::Operation::{CreatePost, CreateRepost, DeletePost, DeleteRepost, UpdatePost},
    },
};

use super::{
    create_post::create_post, create_repost::create_repost, delete_post::delete_post,
    delete_repost::delete_repost,
};

pub async fn post(
    cancellation_token: &CancellationToken,
    store: &mut store::Store,
    http_client: Arc<reqwest::Client>,
    dsts: &[&Account],
) -> Result<()> {
    trace!("post");
    loop {
        trace!("post loop");
        if cancellation_token.is_cancelled() {
            debug!("cancel accepted");
            return Ok(());
        }
        let Some(operation) = store.operations.pop() else {
            trace!("post completed");
            return Ok(());
        };

        let dst = dsts
            .iter()
            .find(|dst| dst.to_account_key() == operation.account_pair().to_dst_key())
            .ok_or_else(|| anyhow!("dst not found"))?;
        let mut dst_client = create_client(http_client.clone(), dst).await?;

        let result = match operation {
            CreatePost(operation) => create_post(store, dst_client.as_mut(), operation).await,
            CreateRepost(operation) => create_repost(store, dst_client.as_mut(), operation).await,
            UpdatePost(_) => {
                warn!("Update is not supported yet");
                Ok(())
            }
            DeletePost(operation) => delete_post(store, dst_client.as_mut(), operation).await,
            DeleteRepost(operation) => delete_repost(store, dst_client.as_mut(), operation).await,
        };
        if let Err(err) = result {
            error!("{:?}", err);
            bail!("post failed");
        }
    }
}
