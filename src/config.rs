use serde::Deserialize;

use crate::{app::AccountKey, protocols::twitter_client};

#[derive(Deserialize)]
#[serde(tag = "protocol")]
pub enum Account {
    #[serde(rename = "atproto")]
    #[serde(rename_all = "camelCase")]
    AtProtocol {
        origin: String,
        identifier: String,
        password: String,
    },
    #[serde(rename = "mastodon")]
    #[serde(rename_all = "camelCase")]
    Mastodon {
        origin: String,
        access_token: String,
    },
    #[serde(rename = "misskey")]
    #[serde(rename_all = "camelCase")]
    Misskey {
        origin: String,
        access_token: String,
    },
    #[serde(rename = "twitter")]
    #[serde(rename_all = "camelCase")]
    Twitter {
        api_key: String,
        api_key_secret: String,
        access_token: String,
        access_token_secret: String,
    },
}

impl Account {
    pub fn to_account_key(&self) -> AccountKey {
        match self {
            Account::AtProtocol {
                origin, identifier, ..
            } => AccountKey {
                origin: origin.clone(),
                identifier: identifier.clone(),
            },
            Account::Mastodon {
                origin,
                access_token,
            } => AccountKey {
                origin: origin.clone(),
                identifier: access_token.clone(),
            },
            Account::Misskey {
                origin,
                access_token,
            } => AccountKey {
                origin: origin.clone(),
                identifier: access_token.clone(),
            },
            Account::Twitter { access_token, .. } => AccountKey {
                origin: twitter_client::ORIGIN.to_string(),
                identifier: access_token.clone(),
            },
        }
    }
}

#[derive(Deserialize)]
pub struct User {
    pub src: Account,
    pub dsts: Vec<Account>,
}

#[derive(Deserialize)]
pub struct Config {
    pub users: Vec<User>,
}
