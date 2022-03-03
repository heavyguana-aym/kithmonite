use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::account::Account;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionRow {
    pub r#type: TransactionType,
    pub amount: Option<Decimal>,
    pub client: u16,
    pub tx: u32,
}

// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Output {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

impl From<Account> for Output {
    fn from(account: Account) -> Self {
        let available: Decimal = account.available.into();
        let held: Decimal = account.held.into();

        Self {
            client: account.client_id.into(),
            available,
            held,
            total: (available + held),
            locked: account.locked,
        }
    }
}
