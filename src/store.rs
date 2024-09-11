pub mod operations;
pub mod user;

use serde::{Deserialize, Serialize};

use crate::app::AccountKey;

use self::{
    operations::{AccountPair, Operation},
    user::{Destination, Source, User},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub users: Vec<User>,
    pub operations: Vec<Operation>,
}

impl Store {
    pub fn get_or_create_user_mut<'a>(&'a mut self, account_key: &AccountKey) -> &'a mut User {
        let idx = self.users.iter().position(|user| {
            user.src.origin == account_key.origin && user.src.identifier == account_key.identifier
        });
        if let Some(idx) = idx {
            return &mut self.users[idx];
        }
        self.users.push(User {
            src: Source {
                origin: account_key.origin.clone(),
                identifier: account_key.identifier.clone(),
                session: None,
                statuses: Vec::default(),
            },
            dsts: Vec::default(),
        });
        self.users.last_mut().unwrap()
    }

    pub fn get_or_create_dst_mut<'a>(
        &'a mut self,
        account_pair: &AccountPair,
    ) -> &'a mut Destination {
        self.get_or_create_user_mut(&account_pair.to_src_key())
            .get_or_create_dst_mut(&account_pair.to_dst_key())
    }
}
