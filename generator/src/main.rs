use clap::Parser;
use kithmonite::cli::{TransactionRow, TransactionType};
use rand::prelude::*;
use rust_decimal::Decimal;

/// Random test transaction history generator. Data correctness is not guaranteed.
/// This serves as a chaos generator to test how the system handles load and invalid
/// orders.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Number of rows to generate.
    #[clap(short, long, default_value_t = 1_000_000)]
    rows: u64,
}

/// Generates a transaction that can refer to past transactions for dispute resolution
fn generate_random_transaction<'a>(
    client_id: u16,
    transaction_history: impl Iterator<Item = &'a TransactionRow>,
) -> TransactionRow {
    let rng = &mut rand::thread_rng();
    let random_transaction_type = [
        TransactionType::Deposit,
        TransactionType::Withdrawal,
        TransactionType::Dispute,
        TransactionType::Resolve,
        TransactionType::Chargeback,
    ]
    .choose(rng)
    .expect("cannot fail because slice is not empty");

    match random_transaction_type {
        TransactionType::Deposit => {
            let transaction_id: u32 = rand::random();
            let deposit_amount = Decimal::new(rand::random::<i64>(), 5).round_dp(4);

            TransactionRow {
                r#type: TransactionType::Deposit,
                amount: Some(deposit_amount),
                client: client_id,
                tx: transaction_id,
            }
        }
        TransactionType::Withdrawal => {
            let transaction_id: u32 = rand::random();
            let withdrawal_amount = Decimal::new(rand::random::<i64>(), 5).round_dp(4);

            TransactionRow {
                r#type: TransactionType::Withdrawal,
                amount: Some(withdrawal_amount),
                client: client_id,
                tx: transaction_id,
            }
        }
        TransactionType::Dispute => {
            let disputed_transaction_id = transaction_history
                .filter(|tx| matches!(tx.r#type, TransactionType::Deposit))
                .map(|tx| tx.tx)
                .choose(rng)
                .unwrap_or_default();

            TransactionRow {
                r#type: TransactionType::Dispute,
                amount: None,
                client: client_id,
                tx: disputed_transaction_id,
            }
        }
        TransactionType::Resolve => {
            let disputed_transaction_id = transaction_history
                .filter(|tx| matches!(tx.r#type, TransactionType::Dispute))
                .map(|tx| tx.tx)
                .choose(rng)
                .unwrap_or_default();

            TransactionRow {
                r#type: TransactionType::Resolve,
                amount: None,
                client: client_id,
                tx: disputed_transaction_id,
            }
        }
        TransactionType::Chargeback => {
            let disputed_transaction_id = transaction_history
                .filter(|tx| matches!(tx.r#type, TransactionType::Chargeback))
                .map(|tx| tx.tx)
                .choose(rng)
                .unwrap_or_default();

            TransactionRow {
                r#type: TransactionType::Chargeback,
                amount: None,
                client: client_id,
                tx: disputed_transaction_id,
            }
        }
    }
}

fn main() {
    let args = Args::parse();

    let client_ids = 0..u16::MAX;
    let transactions_per_client = args.rows / u16::MAX as u64;

    let mut current_client_transactions_buf: Vec<TransactionRow> =
        Vec::with_capacity(transactions_per_client as usize);
    let mut writer = csv::Writer::from_writer(std::io::stdout());

    for client_id in client_ids {
        current_client_transactions_buf.clear();

        for _ in 0..transactions_per_client {
            let row =
                generate_random_transaction(client_id, current_client_transactions_buf.iter());
            writer.serialize(&row).expect("unable to serialize record")
        }
    }
}
