use crate::transaction::{Currency, Transaction, TransactionType};
use csv::{ReaderBuilder, Trim, WriterBuilder};
use log::{debug};
use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::{self};
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub(crate) struct Row {
    #[serde(rename = "Type")]
    r#type: Type,
    #[serde(rename = "Started Date")]
    started_date: String,
    #[serde(rename = "Completed Date")]
    completed_date: Option<String>,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "Amount")]
    amount: Decimal,
    #[serde(rename = "Fee")]
    fee: Decimal,
    #[serde(rename = "Currency")]
    currency: Currency,
    #[serde(rename = "Original Amount")]
    original_amount: Decimal,
    #[serde(rename = "Original Currency")]
    original_currency: Currency,
    #[serde(rename = "Settled Amount")]
    settled_amount: Option<Decimal>,
    #[serde(rename = "Settled Currency")]
    settled_currency: Option<Currency>,
    #[serde(rename = "State")]
    state: State,
    #[serde(rename = "Balance")]
    balance: Option<Decimal>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
enum Type {
    Exchange,
    Transfer,
    Cashback,
    #[serde(rename = "Card Payment")]
    CardPayment,
    Topup,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
enum State {
    Completed
}

#[derive(Debug, Serialize, PartialEq)]
struct Account {
    #[serde(rename = "client")]
    client_id:  u16,
    available:  Decimal,
    held:       Decimal,
    total:      Decimal,
    locked:     bool,
}

/// Reads the file from path into an ordered `Vec<Transaction>`.
async fn deserialize_from_path(path: &PathBuf) -> io::Result<Vec<Row>> {
    let now = std::time::Instant::now();
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        // .delimiter(b';')
        .delimiter(b',')
        .trim(Trim::All)
        .from_path(path)?;
    debug!("ReaderBuilder::from_path done. Elapsed: {:.2?}", now.elapsed());

    let now = std::time::Instant::now();
    let txns: Vec<Row> =
        rdr.deserialize::<Row>()
            .filter_map(|record| record.ok())
            .collect();
    debug!("reader::deserialize done. Elapsed: {:.2?}", now.elapsed());

    Ok(txns)
}

pub(crate) async fn read_exchanges(path: &PathBuf) -> io::Result<Vec<Row>> {
    let txns = deserialize_from_path(path).await?
        .into_iter()
        .filter(|t| t.r#type == Type::Exchange)
        .collect();
    Ok(txns)
}

pub(crate) async fn read_exchanges_in_currency(path: &PathBuf, currency: &Currency) -> io::Result<Vec<Row>> {
    let txns = deserialize_from_path(path).await?
        .into_iter()
        .filter(|t| t.r#type == Type::Exchange)
        .filter(|t| t.currency.eq(currency) || t.description.contains(currency))// "Exchanged to ETH"
        .collect();
    Ok(txns)
}

pub(crate) async fn to_transactions(rows: &Vec<Row>, currency: &Currency) -> io::Result<Vec<Transaction>> {
    let (txns, _): (Vec<Transaction>, Option<&Row>) =
        rows.iter().rev()
            .fold((vec![], None), |(mut acc, prev), row| {
                match prev {
                    None => (acc, Some(row)),
                    Some(prev) => {
                        let txn = prev.to_transaction(None, currency);
                        let txn = row.to_transaction(Some(txn), currency);
                        acc.push(txn);
                        (acc, None)
                    }
                }
            });
    Ok(txns)
}

// 1. Bought Crypto 1 from SEK      (cost in SEK),  sold to SEK      (sales in SEK)
// 2. Bought Crypto 1 from SEK      (cost in SEK),  sold to Crypto 2 (SEK price as sales)
// 3. Bought from Crypto 2 (SEK price as cost),     sold to Crypto 3 (SEK price as sales)
// 4. Bought from Crypto 3 (SEK price as cost),     sold to SEK      (sales in SEK)
impl Row {
    fn to_transaction(&self, txn: Option<Transaction>, currency: &Currency) -> Transaction {
        let mut txn = txn.unwrap_or(Transaction::new());

        // target currency: "BCH", currency: "BCH", description: "Exchanged from SEK"
        if self.currency.eq(currency) && self.description.contains("Exchanged from") {
            debug!("{:?}: Bought {:?} of {:?} ({:?}), incl. fee {:?}", self.started_date, self.amount+self.fee, self.currency, self.description, self.fee);
            txn.r#type = TransactionType::Buy;
            txn.paid_amount = self.amount + self.fee;
            txn.paid_currency = currency.clone();
            txn.date = self.started_date.clone();
        }
        // target currency: "BCH", currency: "BCH", description: "Exchanged to SEK"
        if self.currency.eq(currency) && self.description.contains("Exchanged to") {
            debug!("{:?}: Sold {:?} of {:?} ({:?}), incl. fee {:?}", self.started_date, self.amount+self.fee, self.currency, self.description, self.fee);
            txn.r#type = TransactionType::Sell;
            txn.paid_amount = self.amount + self.fee;
            txn.paid_currency = currency.clone();
            txn.date = self.started_date.clone();
        }
        // target currency: "BCH", currency: "SEK", description: "Exchanged from BCH"
        if self.description.contains("Exchanged from") && self.description.contains(currency) {
            debug!("{:?}: Income of selling is the price of {:?} of {:?} in SEK ({:?}), incl. fee {:?}", self.started_date, self.amount+self.fee, self.currency, self.description, self.fee);
            txn.r#type = TransactionType::Sell;
            txn.exchanged_amount = self.amount + self.fee;
            txn.exchanged_currency = self.currency.clone();
        }
        // target currency: "BCH", currency: "SEK", description: "Exchanged to BCH"
        if self.description.contains("Exchanged to") && self.description.contains(currency) {
            debug!("{:?}: Cost of buying is the price of {:?} of {:?} in SEK ({:?}), incl. fee {:?}", self.started_date, self.amount+self.fee, self.currency, self.description, self.fee);
            txn.r#type = TransactionType::Buy;
            txn.exchanged_amount = self.amount + self.fee;
            txn.exchanged_currency = self.currency.clone();
        }
        if self.description.contains("Vault") {
            txn.is_vault = true;
        }
        txn
    }
}

/// Wraps the `stdout.lock()` in a `csv::Writer` and writes the accounts.
/// The `csv::Writer` is already buffered so there is no need to wrap
/// `stdout.lock()` in a `io::BufWriter`.
pub(crate) async fn print_rows(txns: &Vec<Row>) -> io::Result<()>{
    let stdout = io::stdout();
    let lock = stdout.lock();
    let mut wtr =
        WriterBuilder::new()
            .has_headers(true)
            .from_writer(lock);

    let mut err = None;
    txns.iter().for_each(|t|
        wtr.serialize(t)
            .unwrap_or_else(|e| {
                err = Some(e);
                Default::default()
            })
    );
    err.map_or(Ok(()), Err)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::reader::*;
    use futures::executor::block_on;
    use rust_decimal_macros::dec;
    use std::error::Error;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    #[test]
    fn should_deserialize_from_path() -> Result<(), Box<dyn Error>> {
        /*
         * Given
         */
        let mut file = NamedTempFile::new()?;
        writeln!(file, "Type,Started Date,Completed Date,Description,Amount,Fee,Currency,Original Amount,Original Currency,Settled Amount,Settled Currency,State,Balance
                        Exchange,2022-03-01 16:21:49,2022-03-01 16:21:49,Exchanged to EOS,-900.90603463,-20.36495977,DOGE,-900.90603463,DOGE,,,Completed,1078.7290056
                        Exchange,2022-03-01 16:21:49,2022-03-01 16:21:49,Exchanged from DOGE,50,0,EOS,50,EOS,,,Completed,50
                        Exchange,2021-12-31 17:54:48,2021-12-31 17:54:48,Exchanged to DOGE,-5000.45,-80.15,SEK,-5000.45,SEK,,,Completed,700.27
                        Exchange,2021-12-31 17:54:48,2021-12-31 17:54:48,Exchanged from SEK,2000,0,DOGE,2000,DOGE,,,Completed,2000")?;
        let path = file.path().to_str().unwrap();

        /*
         * When
         */
        let rows = block_on(deserialize_from_path(&PathBuf::from(path)))?;

        /*
         * Then
         */
        let mut iter = rows.into_iter();
        assert_eq!(iter.next(), Some(Row{
            r#type: Type::Exchange,
            started_date: "2022-03-01 16:21:49".to_string(),
            completed_date: Some("2022-03-01 16:21:49".to_string()),
            description: "Exchanged to EOS".to_string(),
            amount: dec!(-900.90603463),
            fee: dec!(-20.36495977),
            currency: "DOGE".to_string(),
            original_amount: dec!(-900.90603463),
            original_currency: "DOGE".to_string(),
            settled_amount: None,
            settled_currency: None,
            state: State::Completed,
            balance: Some(dec!(1078.7290056))
        }));
        assert_eq!(iter.next(), Some(Row{
            r#type: Type::Exchange,
            started_date: "2022-03-01 16:21:49".to_string(),
            completed_date: Some("2022-03-01 16:21:49".to_string()),
            description: "Exchanged from DOGE".to_string(),
            amount: dec!(50),
            fee: dec!(0),
            currency: "EOS".to_string(),
            original_amount: dec!(50),
            original_currency: "EOS".to_string(),
            settled_amount: None,
            settled_currency: None,
            state: State::Completed,
            balance: Some(dec!(50))
        }));
        assert_eq!(iter.next(), Some(Row{
            r#type: Type::Exchange,
            started_date: "2021-12-31 17:54:48".to_string(),
            completed_date: Some("2021-12-31 17:54:48".to_string()),
            description: "Exchanged to DOGE".to_string(),
            amount: dec!(-5000.45),
            fee: dec!(-80.15),
            currency: "SEK".to_string(),
            original_amount: dec!(-5000.45),
            original_currency: "SEK".to_string(),
            settled_amount: None,
            settled_currency: None,
            state: State::Completed,
            balance: Some(dec!(700.27))
        }));
        assert_eq!(iter.next(), Some(Row{
            r#type: Type::Exchange,
            started_date: "2021-12-31 17:54:48".to_string(),
            completed_date: Some("2021-12-31 17:54:48".to_string()),
            description: "Exchanged from SEK".to_string(),
            amount: dec!(2000),
            fee: dec!(0),
            currency: "DOGE".to_string(),
            original_amount: dec!(2000),
            original_currency: "DOGE".to_string(),
            settled_amount: None,
            settled_currency: None,
            state: State::Completed,
            balance: Some(dec!(2000))
        }));
        assert_eq!(iter.next(), None);
        Ok(())
    }

    #[test]
    fn should_parse_to_transactions() -> Result<(), Box<dyn Error>> {
        /*
         * Given
         */
        let rows = vec![
            Row{
                r#type: Type::Exchange,
                started_date: "2022-03-01 16:21:49".to_string(),
                completed_date: Some("2022-03-01 16:21:49".to_string()),
                description: "Exchanged to EOS".to_string(),
                amount: dec!(-900.90603463),
                fee: dec!(-20.36495977),
                currency: "DOGE".to_string(),
                original_amount: dec!(-900.90603463),
                original_currency: "DOGE".to_string(),
                settled_amount: None,
                settled_currency: None,
                state: State::Completed,
                balance: Some(dec!(1078.7290056))
            },
            Row{
                r#type: Type::Exchange,
                started_date: "2022-03-01 16:21:49".to_string(),
                completed_date: Some("2022-03-01 16:21:49".to_string()),
                description: "Exchanged from DOGE".to_string(),
                amount: dec!(50),
                fee: dec!(0),
                currency: "EOS".to_string(),
                original_amount: dec!(50),
                original_currency: "EOS".to_string(),
                settled_amount: None,
                settled_currency: None,
                state: State::Completed,
                balance: Some(dec!(50))
            },
            Row{
                r#type: Type::Exchange,
                started_date: "2021-12-31 17:54:48".to_string(),
                completed_date: Some("2021-12-31 17:54:48".to_string()),
                description: "Exchanged to DOGE".to_string(),
                amount: dec!(-5000.45),
                fee: dec!(-80.15),
                currency: "SEK".to_string(),
                original_amount: dec!(-5000.45),
                original_currency: "SEK".to_string(),
                settled_amount: None,
                settled_currency: None,
                state: State::Completed,
                balance: Some(dec!(700.27))
            },
            Row{
                r#type: Type::Exchange,
                started_date: "2021-12-31 17:54:48".to_string(),
                completed_date: Some("2021-12-31 17:54:48".to_string()),
                description: "Exchanged from SEK".to_string(),
                amount: dec!(2000),
                fee: dec!(0),
                currency: "DOGE".to_string(),
                original_amount: dec!(2000),
                original_currency: "DOGE".to_string(),
                settled_amount: None,
                settled_currency: None,
                state: State::Completed,
                balance: Some(dec!(2000))
            },
            Row{
                r#type: Type::Exchange,
                started_date: "2021-11-11 18:03:13".to_string(),
                completed_date: Some("2021-11-11 18:03:13".to_string()),
                description: "Exchanged to DOGE DOGE Vault".to_string(),
                amount: dec!(-20),
                fee: dec!(0),
                currency: "SEK".to_string(),
                original_amount: dec!(-20),
                original_currency: "SEK".to_string(),
                settled_amount: None,
                settled_currency: None,
                state: State::Completed,
                balance: Some(dec!(500))
            },
            Row{
                r#type: Type::Exchange,
                started_date: "2021-11-11 18:03:13".to_string(),
                completed_date: Some("2021-11-11 18:03:13".to_string()),
                description: "Exchanged from SEK".to_string(),
                amount: dec!(40),
                fee: dec!(-0.06),
                currency: "DOGE".to_string(),
                original_amount: dec!(40),
                original_currency: "DOGE".to_string(),
                settled_amount: None,
                settled_currency: None,
                state: State::Completed,
                balance: Some(dec!(139.94))
            }
        ];
        /*
         * When
         */
        let txns = block_on(to_transactions(&rows, &"DOGE".to_string()))?;

        /*
        * Then
        */
        let mut iter = txns.into_iter();
        assert_eq!(iter.next(), Some(Transaction{
            r#type: TransactionType::Buy,
            paid_currency: "DOGE".to_string(),
            paid_amount: dec!(39.94),
            exchanged_currency: "SEK".to_string(),
            exchanged_amount: dec!(-20),
            date: "2021-11-11 18:03:13".to_string(),
            is_vault: true
        }));
        assert_eq!(iter.next(), Some(Transaction{
            r#type: TransactionType::Buy,
            paid_currency: "DOGE".to_string(),
            paid_amount: dec!(2000),
            exchanged_currency: "SEK".to_string(),
            exchanged_amount: dec!(-5080.60),
            date: "2021-12-31 17:54:48".to_string(),
            is_vault: false
        }));
        assert_eq!(iter.next(), Some(Transaction{
            r#type: TransactionType::Sell,
            paid_currency: "DOGE".to_string(),
            paid_amount: dec!(-921.27099440),
            exchanged_currency: "EOS".to_string(),
            exchanged_amount: dec!(50),
            date: "2022-03-01 16:21:49".to_string(),
            is_vault: false
        }));
        assert_eq!(iter.next(), None);

        Ok(())
    }
}