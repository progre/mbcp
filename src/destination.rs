use anyhow::{anyhow, Result};

use crate::{
    app::commit,
    config::Account,
    protocols::at_proto_client,
    store::{
        self,
        Operation::{Create, Delete, Update},
    },
};

pub async fn post(
    src_origin: &str,
    src_identifier: &str,
    dsts: &[Account],
    store: &mut store::Store,
) -> Result<()> {
    let mut clients = dsts
        .iter()
        .map(|user| match user {
            Account::Mastodon {
                origin: _,
                access_token: _,
            } => {
                todo!();
            }
            Account::AtProtocol {
                origin,
                identifier,
                password,
            } => at_proto_client::Client::new(
                origin.into(),
                reqwest::Client::new(),
                identifier.into(),
                password.into(),
            ),
        })
        .collect::<Vec<_>>();

    for client in &mut clients {
        loop {
            {
                let stored_dst = store
                    .get_or_create_user(src_origin, src_identifier)
                    .get_or_create_dst(client.origin(), &client.identifier);
                let Some(operation) = stored_dst.operations.pop() else {
                    break;
                };
                match operation {
                    Create {
                        src_status_identifier,
                        content,
                        facets,
                        media,
                        external,
                    } => {
                        let identifier = client.post(&content, &facets, media, external).await?;
                        stored_dst.statuses.insert(
                            0,
                            store::DestinationStatus {
                                identifier,
                                src_identifier: src_status_identifier,
                            },
                        );
                    }
                    Update {
                        src_status_identifier: _,
                        content: _,
                        facets: _,
                    } => todo!(),
                    Delete { identifier } => {
                        client.delete(&identifier).await?;
                        let idx = stored_dst
                            .statuses
                            .iter()
                            .position(|status| status.identifier == identifier)
                            .ok_or_else(|| {
                                anyhow!("status not found(identifier={})", identifier)
                            })?;
                        stored_dst.statuses.remove(idx);
                    }
                }
            }
            commit(store).await?;
        }
    }

    Ok(())
}
