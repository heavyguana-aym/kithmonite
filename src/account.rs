use anyhow::{Context, Error as AnyError};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cli::TransactionRow;

// ----------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq, Hash, PartialOrd, Clone, Copy)]
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

#[derive(Debug, Default, PartialEq)]
pub struct TransactionId(u32);

impl From<u32> for TransactionId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

// ----------------------------------------------------------------------------

#[derive(thiserror::Error, Debug)]
pub enum TransactionError {
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
    pub fn overdrawing_add(self, rhs: MonetaryValue) -> Result<MonetaryValue, TransactionError> {
        if self.0 + rhs.0 < Decimal::ZERO {
            return Err(TransactionError::NegativeBalance(self.0 + rhs.0));
        }

        Ok(Self(self.0 + rhs.0))
    }

    /// Calculates self - rhs, returning an error if there's an overdraft
    pub fn overdrawing_sub(self, rhs: MonetaryValue) -> Result<MonetaryValue, TransactionError> {
        if self < rhs {
            return Err(TransactionError::NegativeBalance(self.0 - rhs.0));
        }

        Ok(Self(self.0 - rhs.0))
    }
}

impl TryFrom<Decimal> for MonetaryValue {
    type Error = TransactionError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        if value.is_sign_negative() {
            return Err(TransactionError::NegativeBalance(value));
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

#[derive(PartialEq, Clone, Copy)]
/// Any type of transaction that can happen within the system.
pub enum TransactionKind {
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

#[derive(PartialEq)]
/// Represents an arbitrary transaction.
pub struct Transaction {
    pub id: TransactionId,
    pub kind: TransactionKind,
}

impl Transaction {
    pub fn new(id: impl Into<TransactionId>, kind: TransactionKind) -> Self {
        Self {
            id: id.into(),
            kind,
        }
    }
}

impl TryFrom<TransactionRow> for Transaction {
    type Error = TransactionError;

    fn try_from(row: TransactionRow) -> Result<Self, Self::Error> {
        use crate::cli::TransactionKind::*;
        Ok(Self {
            id: row.tx.into(),
            kind: match row.r#type {
                Deposit => TransactionKind::Deposit(
                    row.amount
                        .context("a deposit transaction should contain an amount")?
                        .try_into()
                        .context("unable to convert deposit amount to a monetary value")?,
                ),
                Withdrawal => TransactionKind::Withdrawal(
                    row.amount
                        .context("a deposit transaction should contain an amount")?
                        .try_into()
                        .context("unable to convert withdrawal amount to a monetary value")?,
                ),
                Dispute => TransactionKind::Dispute,
                Resolve => TransactionKind::Resolve,
                Chargeback => TransactionKind::Chargeback,
            },
        })
    }
}

// ----------------------------------------------------------------------------

#[derive(Debug, Default)]
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
    pub fn new(client_id: impl Into<ClientId>) -> Self {
        Self {
            client_id: client_id.into(),
            ..Default::default()
        }
    }
}

// ----------------------------------------------------------------------------

pub type AccountOperationResult = Result<(), TransactionError>;

impl Account {
    /// Adds a given amount of money to the account's available funds.
    pub fn deposit(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_add(amount)?;
        Ok(())
    }

    /// Removes a given amount of money from the account's available funds
    pub fn withdraw(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_sub(amount)?;
        Ok(())
    }

    /// Freezes a given amount of money while a dispute is being solved.
    pub fn hold(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_sub(amount)?;
        self.held = self.held.overdrawing_add(amount)?;

        Ok(())
    }

    /// Un-freeze a given amount of money, typically when a dispute is solved.
    pub fn release(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.check_lock()?;

        self.available = self.available.overdrawing_add(amount)?;
        self.held = self.held.overdrawing_sub(amount)?;

        Ok(())
    }

    /// Removes funds from the held funds, typically when a chargeback occurs.
    pub fn chargeback(&mut self, amount: MonetaryValue) -> AccountOperationResult {
        self.locked = true;

        self.held = self.held.overdrawing_sub(amount)?;
        Ok(())
    }

    /// Returns an `AccountLocked` error if the account is locked.
    fn check_lock(&self) -> AccountOperationResult {
        if self.locked {
            return Err(TransactionError::AccountLocked);
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
        let mut account = Account::new(1);
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert_eq!(account.available, deposit);
    }

    #[test]
    fn withdraw_funds() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);
        let withdrawal = money!(1.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.withdraw(withdrawal).is_ok());
        assert_eq!(account.available, money!(1.0));
    }

    #[test]
    fn deposit_negative_funds() {
        let mut account = Account::new(1);
        let deposit = MonetaryValue(Decimal::try_from(-2.0).expect("-2.0 is a valid decimal"));

        assert!(account.deposit(deposit).is_err());
        assert_eq!(account.available, money!(0.0));
    }

    #[test]
    fn withdraw_more_than_available() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);
        let withdrawal = money!(3.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.withdraw(withdrawal).is_err());
        assert_eq!(account.available, money!(2.0));
    }

    #[test]
    fn dispute_deposit() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.hold(deposit).is_ok());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, money!(2.0));
    }

    #[test]
    fn resolve_dispute() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.hold(deposit).is_ok());
        assert!(account.release(deposit).is_ok());
        assert_eq!(account.available, money!(2.0));
        assert_eq!(account.held, money!(0.0));
    }

    #[test]
    fn chargeback_dispute() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.hold(deposit).is_ok());
        assert!(account.chargeback(deposit).is_ok());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, money!(0.0));
        assert!(account.locked);
    }

    #[test]
    fn spend_held_funds() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.hold(deposit).is_ok());
        assert!(account.withdraw(deposit).is_err());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, deposit);
        assert!(!account.locked);
    }

    #[test]
    fn spend_resolved_funds() {
        let mut account = Account::new(1);
        let deposit = money!(2.0);

        assert!(account.deposit(deposit).is_ok());
        assert!(account.hold(deposit).is_ok());
        assert!(account.release(deposit).is_ok());
        assert!(account.withdraw(deposit).is_ok());
        assert_eq!(account.available, money!(0.0));
        assert_eq!(account.held, money!(0.0));
        assert!(!account.locked);
    }
}
