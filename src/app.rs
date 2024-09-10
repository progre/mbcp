use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Ok, Result};
use futures::future::join_all;
use tokio::{spawn, time::sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, trace};

use crate::{
    config,
    database::Database,
    operations::destination::post,
    sources::source::{get, retain_all_dst_statuses},
    store,
};

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct AccountKey {
    pub origin: String,
    pub identifier: String,
}

pub async fn do_main_task(
    cancellation_token: &CancellationToken,
    config: &config::Config,
    store: &mut store::Store,
) -> Result<()> {
    trace!("do_main_task");
    let http_client = Arc::new(reqwest::Client::new());
    let store = Mutex::new(store);
    let users = config.users.iter();
    let futures = users.map(|config_user| get(&http_client, config_user, &store));
    for result in join_all(futures).await {
        result?;
    }
    let store = store.into_inner().unwrap();
    if cancellation_token.is_cancelled() {
        debug!("cancel accepted");
        return Ok(());
    }
    post(
        cancellation_token,
        store,
        http_client.clone(),
        &config
            .users
            .iter()
            .flat_map(|user| &user.dsts)
            .collect::<Vec<_>>(),
    )
    .await?;
    if cancellation_token.is_cancelled() {
        debug!("cancel accepted");
        return Ok(());
    }
    if store.operations.is_empty() {
        retain_all_dst_statuses(store).await?;
    }
    trace!("do_main_task completed");

    Ok(())
}

pub async fn app(database: impl Database) -> Result<()> {
    let cancellation_token = CancellationToken::new();
    spawn({
        let cancellation_token = cancellation_token.clone();
        async move {
            sleep(Duration::from_secs(60 + 20)).await;
            debug!("cancel request");
            cancellation_token.cancel();
        }
    });
    spawn(async move {
        let config = database.config().await?;
        let mut store = database.fetch().await.unwrap_or_default();

        let main_result = do_main_task(&cancellation_token, &config, &mut store).await;

        let commit_result = database.commit(&store).await;
        if let Err(main_error) = main_result {
            if let Err(commit_error) = commit_result {
                error!("commit error: {:?}", commit_error);
            }
            return Err(main_error);
        }

        commit_result
    })
    .await?
}
