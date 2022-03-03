use anyhow::{Context, Result as AnyResult};
use csv::Trim;
use kithmonite::{
    account::{Account, ClientId, Error, Transaction},
    cli::{self, TransactionRow},
};
use std::{collections::HashMap, env, fs::File, time::Instant};

fn accounts_from_transactions(
    transactions: &mut impl Iterator<Item = TransactionRow>,
) -> Result<impl Iterator<Item = Account>, Error> {
    let transactions_index = transactions
        .try_fold(
            HashMap::<ClientId, Vec<Transaction>>::new(),
            |mut index, transaction| -> AnyResult<_> {
                let entry = index.entry(transaction.client.into()).or_default();

                if let Ok(transaction) = Transaction::try_from(transaction) {
                    entry.push(transaction);
                }

                Ok(index)
            },
        )
        .context("unable to build user account index")?;

    let client_accounts = transactions_index
        .into_iter()
        .map(|(client_id, transactions)| Account::from_transactions(client_id, transactions))
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(client_accounts.into_iter())
}

fn main() -> AnyResult<()> {
    let timer = Instant::now();

    let transactions_file_path = env::args().nth(1).context("an input file is required")?;
    let transactions_file =
        File::open(transactions_file_path).context("unable to read the transactions file")?;

    let mut reader = csv::ReaderBuilder::new()
        .trim(Trim::All)
        .from_reader(transactions_file);

    let mut extracted_transactions = reader
        .deserialize::<cli::TransactionRow>()
        .map(|res| res.context("unable to deserialize transaction"))
        .collect::<AnyResult<Vec<_>>>()?
        .into_iter();

    let extraction_duration = Instant::now().duration_since(timer);
    println!("time elapsed during extraction: {:#?}", extraction_duration);

    let accounts = accounts_from_transactions(&mut extracted_transactions)?.map(cli::Output::from);

    let total_duration = Instant::now().duration_since(timer);
    println!("time elapsed: {:#?}", total_duration);

    let mut writer = csv::Writer::from_writer(std::io::stdout());
    accounts.for_each(|account| {
        writer
            .serialize(&account)
            .expect("unable to serialize record")
    });

    Ok(())
}
