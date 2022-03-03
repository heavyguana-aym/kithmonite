use std::collections::HashMap;

use anyhow::{Context, Error as AnyError};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cli::TransactionRow;

// ----------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq, Hash, Serialize, PartialOrd)]
pub struct ClientId(u16);

impl From<u16> for ClientId {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<ClientId> for u16 {
    fn from(id: ClientId) -> Self {
        id.0
    }
}

// ----------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq, Hash, Serialize, PartialOrd, Clone, Copy)]
pub struct TransactionId(u32);

impl From<u32> for TransactionId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

// ----------------------------------------------------------------------------

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("a monetary value cannot be negative: {0:?}")]
    NegativeBalance(Decimal),
    #[error("account is locked")]
    AccountLocked,
    #[error(transparent)]
    Other(#[from] AnyError),
}

// ----------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Serialize, Clone, Copy)]
/// Represents an arbitrary, positive amount of money.
/// This type strives to be as restrictive as possible to avoid potential errors.
pub struct MonetaryValue(Decimal);

impl MonetaryValue {
    /// Calculates self + rhs, returning an error if there's an overdraft
    pub fn overdrawing_add(self, rhs: MonetaryValue) -> Result<MonetaryValue, Error> {
        if self.0 + rhs.0 < Decimal::ZERO {
            return Err(Error::NegativeBalance(self.0 + rhs.0));
        }

        Ok(Self(self.0 + rhs.0))
    }

    /// Calculates self - rhs, returning an error if there's an overdraft
    pub fn overdrawing_sub(self, rhs: MonetaryValue) -> Result<MonetaryValue, Error> {
        if self < rhs {
            return Err(Error::NegativeBalance(self.0 - rhs.0));
        }

        Ok(Self(self.0 - rhs.0))
    }
}

impl TryFrom<Decimal> for MonetaryValue {
    type Error = Error;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        if value.is_sign_negative() {
            return Err(Error::NegativeBalance(value));
        }

        Ok(Self(value.round_dp(4)))
    }
}

impl From<MonetaryValue> for Decimal {
    fn from(value: MonetaryValue) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for MonetaryValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let decimal: Decimal = Deserialize::deserialize(deserializer)?;
        decimal
            .try_into()
            .context("unable to convert a `Decimal` to a monetary value")
            .map_err(serde::de::Error::custom)
    }
}

// ----------------------------------------------------------------------------

#[derive(Debug, PartialEq, Serialize, PartialOrd)]
/// Any type of transaction that can happen within the system.
pub enum TransactionType {
    /// A credit to the client's asset account.
    Deposit(MonetaryValue),
    /// A debit to the client's asset account.
    Withdrawal(MonetaryValue),
    /// A client's claim that a transaction was erroneous and should be reversed.
    Dispute,
    /// A resolution to a dispute, releasing the associated held funds.
    Resolve,
    /// The final state of a dispute that represents the client reversing a transaction.
    Chargeback,
}

#[derive(Debug, PartialEq, Serialize, PartialOrd)]
/// Represents an arbitrary transaction.
pub struct Transaction {
    id: TransactionId,
    transaction: TransactionType,
}

impl TryFrom<TransactionRow> for Transaction {
    type Error = Error;

    fn try_from(row: TransactionRow) -> Result<Self, Self::Error> {
        use crate::cli::TransactionType::*;
        Ok(Self {
            id: row.tx.into(),
            transaction: match row.r#type {
                Deposit => TransactionType::Deposit(
                    row.amount
                        .context("a deposit transaction should contain an amount")?
                        .try_into()
                        .context("unable to convert deposit amount to a monetary value")?,
                ),
                Withdrawal => TransactionType::Withdrawal(
                    row.amount
                        .context("a deposit transaction should contain an amount")?
                        .try_into()
                        .context("unable to convert withdrawal amount to a monetary value")?,
                ),
                Dispute => TransactionType::Dispute,
                Resolve => TransactionType::Resolve,
                Chargeback => TransactionType::Chargeback,
            },
        })
    }
}

// ----------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, PartialOrd)]
/// Represents the state of an account at a given point in time.
pub struct Account {
    pub client_id: ClientId,
    /// Total funds that are available for trading, staking and withdrawal.
    pub available: MonetaryValue,
    /// Total funds that are held for dispute.
    pub held: MonetaryValue,
    /// Whether the account is locked. No transactions can happen on a locked account.
    pub locked: bool,
}

impl Account {
    pub fn from_transactions<'a>(
        client_id: ClientId,
        transactions: Vec<Transaction>,
    ) -> Result<Self, Error> {
        let transactions_by_id: HashMap<TransactionId, Vec<&Transaction>> = transactions
            .iter()
            .fold(Default::default(), |mut index, transaction| {
                index.entry(transaction.id).or_default().push(transaction);
                index
            });

        transactions
            .iter()
            .try_fold(Self::new(client_id), |mut account, transaction| {
                use TransactionType::*;

                let disputed_amount =
                    if matches!(transaction.transaction, Dispute | Resolve | Chargeback) {
                        transactions_by_id.get(&transaction.id).and_then(|txs| {
                            txs.into_iter().find_map(|tx| match tx.transaction {
                                Deposit(amount) => Some(amount),
                                _ => None,
                            })
                        })
                    } else {
                        None
                    };

                match transaction.transaction {
                    Deposit(amount) => {
                        let _ = account.deposit(amount);

                        Ok(())
                    }
                    Withdrawal(amount) => {
                        let _ = account.withdraw(amount);

                        Ok(())
                    }
                    Dispute => {
                        if let Some(disputed_amount) = disputed_amount {
                            account
                                .dispute(disputed_amount)
                                .context("unable to dispute a transaction")
                                .map_err(Error::Other)
                        } else {
                            Ok(())
                        }
                    }
                    Resolve => {
                        if let Some(disputed_amount) = disputed_amount {
                            account
                                .resolve(disputed_amount)
                                .context("unable to resolve dispute")
                                .map_err(Error::Other)
                        } else {
                            Ok(())
                        }
                    }
                    Chargeback => {
                        if let Some(disputed_amount) = disputed_amount {
                            account
                                .chargeback(disputed_amount)
                                .context("unable to chargeback")
                                .map_err(Error::Other)
                        } else {
                            Ok(())
                        }
                    }
                }?;

                Ok(account)
            })
    }

    pub fn new(client_id: ClientId) -> Self {
        Self {
            client_id,
            ..Default::default()
        }
    }
}

// ----------------------------------------------------------------------------

type AccountOperationResult = Result<(), Error>;

impl Account {
    fn deposit(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_add(amount)?;
        Ok(())
    }

    fn withdraw(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_sub(amount)?;
        Ok(())
    }

    fn dispute(&mut self, disputed_amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_sub(disputed_amount)?;
        self.held = self.held.overdrawing_add(disputed_amount)?;

        Ok(())
    }

    fn resolve(&mut self, disputed_amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_add(disputed_amount)?;
        self.held = self.held.overdrawing_sub(disputed_amount)?;

        Ok(())
    }

    fn chargeback(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.locked = true;

        self.held = self.held.overdrawing_sub(amount)?;
        Ok(())
    }

    /// Returns an `AccountLocked` error if the account is locked.
    fn check_lock(&self) -> AccountOperationResult {
        if self.locked {
            return Err(Error::AccountLocked);
        }

        Ok(())
    }
}

// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{Account, MonetaryValue};

    macro_rules! money {
        ($dec:expr) => {
            MonetaryValue(rust_decimal_macros::dec!($dec))
        };
    }

    #[test]
    fn add_funds() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert_eq!(account.available, deposit);
    }

    #[test]
    fn withdraw_funds() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);
        let withdrawal = money!(1.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.withdraw(withdrawal).is_ok());
        assert_eq!(account.available, money!(1.0));
    }

    #[test]
    fn deposit_negative_funds() {
        let mut account = Account::new(1.into());
        let deposit = MonetaryValue(Decimal::try_from(-2.0).expect("-2.0 is a valid decimal"));

        assert!(account.deposit(deposit).is_err());
        assert_eq!(account.available, money!(0.0));
    }

    #[test]
    fn withdraw_more_than_available() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);
        let withdrawal = money!(3.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.withdraw(withdrawal).is_err());
        assert_eq!(account.available, money!(2.0));
    }

    #[test]
    fn dispute_deposit() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.dispute(deposit).is_ok());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, money!(2.0));
    }

    #[test]
    fn resolve_dispute() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.dispute(deposit).is_ok());
        assert!(account.resolve(deposit).is_ok());
        assert_eq!(account.available, money!(2.0));
        assert_eq!(account.held, money!(0.0));
    }

    #[test]
    fn chargeback_dispute() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.dispute(deposit).is_ok());
        assert!(account.chargeback(deposit).is_ok());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, money!(0.0));
        assert!(account.locked);
    }

    #[test]
    fn spend_held_funds() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.dispute(deposit).is_ok());
        assert!(account.withdraw(deposit).is_err());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, deposit);
        assert!(!account.locked);
    }

    #[test]
    fn spend_resolved_funds() {
        let mut account = Account::new(1.into());
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.dispute(deposit).is_ok());
        assert!(account.resolve(deposit).is_ok());
        assert!(account.withdraw(deposit).is_ok());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, money!(0.0));
        assert!(!account.locked);
    }
}
