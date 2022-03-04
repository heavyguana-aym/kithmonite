use std::collections::HashMap;

use crate::account::{Account, ClientId, Transaction, TransactionError};

pub struct AccountLog {
    state: Account,
    history: Vec<Transaction>,
}

impl AccountLog {
    pub fn new(client_id: impl Into<ClientId>) -> Self {
        Self {
            state: Account::new(client_id),
            history: Vec::new(),
        }
    }
}

// ----------------------------------------------------------------------------

#[derive(thiserror::Error, Debug)]
pub enum PaymentProcessingError {
    #[error("an error occured during the underlying transaction")]
    TransactionError(#[from] TransactionError),
    #[error("multiple disputes on the same transaction")]
    DisputeAlreadyExists,
    #[error("relevant transaction is not in dispute")]
    NoDispute,
}

type PaymentProcessingResult = Result<(), PaymentProcessingError>;

// ----------------------------------------------------------------------------

/// A state machine that takes client transactions and incrementally builds accounts
/// from the transaction history.
pub struct PaymentProcessor {
    accounts: HashMap<ClientId, AccountLog>,
}

impl PaymentProcessor {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
        }
    }

    /// Takes a transaction and applies it to the relevant customer account. The
    /// transaction will be persisted until the processor is dropped.
    pub fn process<I>(&mut self, client_id: I, transaction: Transaction) -> PaymentProcessingResult
    where
        I: Into<ClientId> + Copy,
    {
        let account = self
            .accounts
            .entry(client_id.into())
            .or_insert_with(|| AccountLog::new(client_id));

        use crate::account::TransactionKind::*;
        match transaction.kind {
            Deposit(amount) => {
                account.state.deposit(amount)?;
                account.history.push(transaction);
            }
            Withdrawal(amount) => {
                account.state.withdraw(amount)?;
                account.history.push(transaction);
            }
            Dispute => {
                let dispute_exists = account
                    .history
                    .iter()
                    .any(|tx| tx.id == transaction.id && matches!(tx.kind, Dispute));

                if dispute_exists {
                    return Err(PaymentProcessingError::DisputeAlreadyExists);
                }

                let disputed_transaction =
                    account.history.iter().find(|tx| tx.id == transaction.id);

                if let Some(Deposit(disputed_amount)) = disputed_transaction.map(|tx| tx.kind) {
                    account.state.hold(disputed_amount)?;
                    account.history.push(transaction);
                };
            }
            Resolve => {
                let dispute_exists = account
                    .history
                    .iter()
                    .any(|tx| tx.id == transaction.id && matches!(tx.kind, Dispute));

                if !dispute_exists {
                    return Err(PaymentProcessingError::NoDispute);
                }

                let disputed_transaction =
                    account.history.iter().find(|tx| tx.id == transaction.id);

                if let Some(Deposit(disputed_amount)) = disputed_transaction.map(|tx| tx.kind) {
                    account.state.release(disputed_amount)?;
                    account.history.push(transaction);
                };
            }
            Chargeback => {
                let dispute_exists = account
                    .history
                    .iter()
                    .any(|tx| tx.id == transaction.id && matches!(tx.kind, Dispute));

                if !dispute_exists {
                    return Err(PaymentProcessingError::NoDispute);
                }

                let disputed_transaction =
                    account.history.iter().find(|tx| tx.id == transaction.id);

                if let Some(Deposit(disputed_amount)) = disputed_transaction.map(|tx| tx.kind) {
                    account.state.chargeback(disputed_amount)?;
                    account.history.push(transaction);
                };
            }
        };

        Ok(())
    }

    /// Retrieves the account log of the specified client
    pub fn account_log(&self, client_id: impl Into<ClientId>) -> Option<&AccountLog> {
        self.accounts.get(&client_id.into())
    }

    /// Returns an iterator over all existing accounts
    pub fn accounts(self) -> impl Iterator<Item = Account> {
        self.accounts.into_iter().map(|(_, log)| log.state)
    }
}

// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::PaymentProcessor;
    use crate::account::{Transaction, TransactionKind};

    macro_rules! tx {
        ($id:expr, deposit, $amount:expr) => {
            Transaction::new($id, TransactionKind::Deposit($amount))
        };
        ($id:expr, withdrawal, $amount:expr) => {
            Transaction::new($id, TransactionKind::Withdrawal($amount))
        };
        ($id:expr, dispute) => {
            Transaction::new($id, TransactionKind::Dispute)
        };
        ($id:expr, resolve) => {
            Transaction::new($id, TransactionKind::Resolve)
        };
        ($id:expr, chargeback) => {
            Transaction::new($id, TransactionKind::Chargeback)
        };
    }

    #[test]
    fn deposit() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, deposit_value);
    }

    #[test]
    fn withdrawal() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor
            .process(1, tx!(1, withdrawal, deposit_value))
            .is_ok());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, zero);
    }

    #[test]
    fn dispute() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor.process(1, tx!(1, dispute)).is_ok());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, zero);
        assert_eq!(account.state.held, deposit_value);
    }

    #[test]
    fn dispute_already_disputed_tx() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor.process(1, tx!(1, dispute)).is_ok());
        assert!(processor.process(1, tx!(1, dispute)).is_err());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, zero);
        assert_eq!(account.state.held, deposit_value);
    }

    #[test]
    fn withdraw_insufficient_funds() {
        let mut processor = PaymentProcessor::new();
        let withdrawal_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor
            .process(1, tx!(1, withdrawal, withdrawal_value))
            .is_err());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, zero);
    }

    #[test]
    fn resolve_dispute() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor.process(1, tx!(1, dispute)).is_ok());
        assert!(processor.process(1, tx!(1, resolve)).is_ok());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, deposit_value);
        assert_eq!(account.state.held, zero);
    }

    #[test]
    fn chargeback_dispute() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor.process(1, tx!(1, dispute)).is_ok());
        assert!(processor.process(1, tx!(1, chargeback)).is_ok());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, zero);
        assert_eq!(account.state.held, zero);
    }

    #[test]
    fn resolve_no_dispute() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor.process(1, tx!(1, resolve)).is_err());

        let account = processor
            .account_log(1)
            .expect("an account should have been created");

        assert_eq!(account.state.available, deposit_value);
        assert_eq!(account.state.held, zero);
    }

    #[test]
    fn multiple_clients() {
        let mut processor = PaymentProcessor::new();
        let deposit_value = dec!(1.0).try_into().expect("1.0 is a decimal");
        let zero = dec!(0.0).try_into().expect("0.0 is a decimal");

        assert!(processor.process(1, tx!(1, deposit, deposit_value)).is_ok());
        assert!(processor.process(2, tx!(1, deposit, deposit_value)).is_ok());

        let account_1 = processor
            .account_log(1)
            .expect("an account should have been created");
        let account_2 = processor
            .account_log(2)
            .expect("an account should have been created");

        assert_eq!(account_1.state.available, deposit_value);
        assert_eq!(account_1.state.held, zero);
        assert_eq!(account_2.state.available, deposit_value);
        assert_eq!(account_2.state.held, zero);
    }
}
