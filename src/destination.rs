use std::collections::HashMap;

use anyhow::Result;

use crate::{
    app::AccountKey,
    database::Database,
    protocols::Client,
    store::{
        self,
        operations::Operation::{Create, Delete, Update},
    },
};

fn to_dst_identifier<'a>(src_identifier: &str, store: &'a store::Store) -> Option<&'a str> {
    Some(
        store
            .users
            .iter()
            .flat_map(|user| &user.dsts)
            .flat_map(|dst| &dst.statuses)
            .find(|dst| dst.src_identifier == src_identifier)?
            .identifier
            .as_str(),
    )
}

pub async fn post_operation(
    store: &mut store::Store,
    dst_client: &mut dyn Client,
    operation: store::operations::Operation,
) -> Result<()> {
    match operation {
        Create(create) => {
            let store::operations::CreatingStatus {
                src_identifier,
                content,
                facets,
                reply_src_identifier,
                media,
                external,
                created_at,
            } = create.status;
            let reply_identifier =
                reply_src_identifier.and_then(|reply| to_dst_identifier(&reply, &*store));
            let dst_identifier = dst_client
                .post(
                    &content,
                    &facets,
                    reply_identifier,
                    media,
                    external,
                    &created_at,
                )
                .await?;
            store
                .get_or_create_dst_mut(&create.account_pair)
                .statuses
                .insert(
                    0,
                    store::user::DestinationStatus {
                        identifier: dst_identifier,
                        src_identifier,
                    },
                );
        }
        Update(store::operations::UpdateOperation {
            account_pair: _,
            src_identifier: _,
            content: _,
            facets: _,
        }) => todo!(),
        Delete(store::operations::DeleteOperation {
            account_pair: _,
            src_identifier,
        }) => {
            if let Some(dst_identifier) = to_dst_identifier(&src_identifier, &*store) {
                dst_client.delete(dst_identifier).await?;
            }
        }
    }

    Ok(())
}

pub async fn post(
    database: &impl Database,
    store: &mut store::Store,
    dst_clients_map: &mut HashMap<AccountKey, Vec<Box<dyn Client>>>,
) -> Result<()> {
    // WTF: DynamoDB の連続アクセス不能問題が解消するまで連続作業を絞る
    for _ in 0..2 {
        let Some(operation) = store.operations.pop() else {
            break;
        };

        let dst_client = dst_clients_map
            .get_mut(&operation.account_pair().to_src_key())
            .unwrap()
            .iter_mut()
            .find(|dst_client| dst_client.to_account_key() == operation.account_pair().to_dst_key())
            .unwrap();

        post_operation(store, dst_client.as_mut(), operation).await?;
        database.commit(store).await?;
    }

    Ok(())
}
