use anyhow::{Context, Result};
use csv::Trim;
use kithmonite::{account::Transaction, cli, processor::PaymentProcessor};
use std::env;

fn main() -> Result<()> {
    let transactions_file_path = env::args().nth(1).context("an input file is required")?;

    let mut reader = csv::ReaderBuilder::new()
        .trim(Trim::All)
        .from_path(transactions_file_path)
        .context("unable to create csv reader for given file")?;

    let mut payment_processor = PaymentProcessor::new();

    for row in reader.deserialize::<cli::TransactionRow>() {
        let transaction = row.context("unable to deserialize transaction")?;
        let client_id = transaction.client;
        if let Ok(tx) = Transaction::try_from(transaction) {
            let _ = payment_processor.process(client_id, tx);
        }
    }

    let accounts = payment_processor.accounts().map(cli::Output::from);

    let mut writer = csv::Writer::from_writer(std::io::stdout());

    for account in accounts {
        writer
            .serialize(&account)
            .context("unable to serialize record")?
    }

    Ok(())
}
