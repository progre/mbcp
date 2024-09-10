use tracing::warn;

use super::source::Operation;
use crate::{
    app::AccountKey,
    store::{
        self,
        operations::Operation::{CreatePost, CreateRepost, DeletePost, DeleteRepost, UpdatePost},
    },
};

fn to_store_operations(
    dst_account_keys: &[AccountKey],
    operations: &[Operation],
    src_account_key: &AccountKey,
) -> Vec<store::operations::Operation> {
    dst_account_keys
        .iter()
        .flat_map(|dst_account_key| {
            let account_pair = store::operations::AccountPair::from_keys(
                src_account_key.clone(),
                dst_account_key.clone(),
            );

            operations
                .iter()
                .map(|operation| operation.to_store(account_pair.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/** 投稿は降順で、それ以外は末尾に積む */
fn sort_operations(operations: &mut [store::operations::Operation]) {
    operations.sort_by_key(|operation| -match operation {
        CreatePost(content) => content.status.created_at.timestamp_micros(),
        CreateRepost(content) => content.status.created_at.timestamp_micros(),
        UpdatePost(_) | DeleteRepost(_) => i64::MAX - 1,
        DeletePost(_) => i64::MAX,
    });
}

fn to_update_post_operation_status(
    src_operation: &Operation,
) -> Option<&store::operations::UpdatePostOperationStatus> {
    if let Operation::UpdatePost(status) = src_operation {
        Some(status)
    } else {
        None
    }
}

fn to_delete_post_operation_status(
    src_operation: &Operation,
) -> Option<&store::operations::DeletePostOperationStatus> {
    if let Operation::DeletePost(status) = src_operation {
        Some(status)
    } else {
        None
    }
}

fn to_delete_repost_operation_status(
    src_operation: &Operation,
) -> Option<&store::operations::DeleteRepostOperationStatus> {
    if let Operation::DeleteRepost(status) = src_operation {
        Some(status)
    } else {
        None
    }
}

fn create_operation_target_state(
    content: &store::operations::CreatePostOperation,
) -> (AccountKey, &str) {
    (
        content.account_pair.to_src_key(),
        &content.status.src_identifier,
    )
}

pub fn merge_operations(
    store: &mut store::Store,
    dst_account_keys: &[AccountKey],
    src_account_key: &AccountKey,
    src_operations: &[Operation],
) {
    let mut new_operations = to_store_operations(dst_account_keys, src_operations, src_account_key);

    let operations = &mut store.operations;

    // 投稿の更新
    src_operations
        .iter()
        .filter_map(to_update_post_operation_status)
        .for_each(|_| warn!("もし create が未送信なら、create を書き換える必要がある"));
    // 投稿の削除を適用
    let deleting_post_full_identifiers: Vec<_> = src_operations
        .iter()
        .filter_map(to_delete_post_operation_status)
        .map(|status| (src_account_key.clone(), status.src_identifier.as_str()))
        .collect();
    operations.retain(|dst_operation| match dst_operation {
        CreatePost(content) => {
            let operation_full_identifier = create_operation_target_state(content);
            !deleting_post_full_identifiers.contains(&operation_full_identifier)
        }
        CreateRepost(content) => {
            let operation_full_identifier = (
                content.account_pair.to_src_key(),
                content.status.target_src_identifier.as_str(),
            );
            !deleting_post_full_identifiers.contains(&operation_full_identifier)
        }
        UpdatePost(_) | DeletePost(_) | DeleteRepost(_) => true,
    });
    // repost の削除を適用
    let deleting_repost_full_identifiers: Vec<_> = src_operations
        .iter()
        .filter_map(to_delete_repost_operation_status)
        .map(|status| (src_account_key.clone(), status.src_identifier.as_str()))
        .collect();
    operations.retain(|dst_operation| match dst_operation {
        CreateRepost(content) => {
            let operation_full_identifier = (
                content.account_pair.to_src_key(),
                content.status.src_identifier.as_str(),
            );
            !deleting_repost_full_identifiers.contains(&operation_full_identifier)
        }
        CreatePost(_) | UpdatePost(_) | DeletePost(_) | DeleteRepost(_) => true,
    });

    operations.append(&mut new_operations);
    sort_operations(operations);
}
