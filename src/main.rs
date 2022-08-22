use fixed::types::I50F14;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::hash::Hash;

// You wanted precision to 0.0001,
// but you'll get precision to 0.000061.
// Fixed point chosen so that operations are deterministic across
// all architectures, and to retain associativity/commutativity
type Currency = I50F14;

#[derive(Serialize, Deserialize, Debug, PartialEq, Hash, Eq, Clone, Copy, Default)]
#[serde(transparent)]
struct ClientId(u16);

#[derive(Serialize, Deserialize, Debug, PartialEq, Hash, Eq, Clone, Copy, Default)]
#[serde(transparent)]
struct TxId(u32);

#[repr(u8)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum TxType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    ChargeBack,
}

// Dedicated struct to deserialize just so that the csv library
// doesn't try to find key/value pairs instead of just values.
#[derive(Deserialize, Debug)]
struct InputTx(TxType, u16, u32, Option<Currency>);

#[derive(Deserialize, Debug)]
struct Tx {
    tx_type: TxType,
    cid: ClientId,
    tid: TxId,
    amount: Currency,
}

impl From<InputTx> for Tx {
    fn from(input: InputTx) -> Self {
        Tx {
            tx_type: input.0,
            cid: ClientId(input.1),
            tid: TxId(input.2),
            amount: input.3.unwrap_or(Currency::from_num(0)),
        }
    }
}

// Only for testing, normally the tx is created using From<InputTx>
#[cfg(test)]
impl Tx {
    fn new(ty: TxType, cid: u16, tid: u32, amount: Currency) -> Self {
        Tx {
            tx_type: ty,
            cid: ClientId(cid),
            tid: TxId(tid),
            amount,
        }
    }
}

#[derive(Default)]
struct ClientState {
    available: Currency,
    held: Currency,
    locked: bool,
    history: HashMap<TxId, Tx>,
    disputed: HashMap<TxId, Tx>,
}

// bit hacky as this is limiting to only string output, but good enough for a demo cli tool.
fn precision4_serialize_currency<S>(currency: &Currency, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_str(&format!("{:.4}", currency))
}

#[derive(Serialize)]
struct ClientOutputState {
    cid: ClientId,
    #[serde(serialize_with = "precision4_serialize_currency")]
    available: Currency,
    #[serde(serialize_with = "precision4_serialize_currency")]
    held: Currency,
    #[serde(serialize_with = "precision4_serialize_currency")]
    total: Currency,
    locked: bool,
}

impl ClientOutputState {
    // Not a proper trait... but need the second argument
    fn from(input: ClientState, cid: ClientId) -> Self {
        ClientOutputState {
            cid,
            available: input.available,
            held: input.held,
            total: input.available + input.held,
            locked: input.locked,
        }
    }
}

#[derive(Default)]
struct AppState {
    clients: HashMap<ClientId, ClientState>,
}

#[derive(Debug)]
struct BasicError {
    desc: &'static str,
}

impl BasicError {
    fn new(desc: &'static str) -> Box<Self> {
        Box::new(BasicError { desc })
    }
}

impl Display for BasicError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", self.desc)
    }
}

impl Error for BasicError {
    fn description(&self) -> &str {
        self.desc
    }

    fn cause(&self) -> Option<&dyn Error> {
        None
    }
}

fn execute_transaction(app_state: &mut AppState, tx: Tx) {
    let mut client_entry = app_state.clients.entry(tx.cid).or_default();

    match &tx.tx_type {
        TxType::Deposit => {
            client_entry.available += tx.amount;
        }
        TxType::Withdrawal => {
            if client_entry.available >= tx.amount
            {
                client_entry.available -= tx.amount;
            }
            else
            {
                eprintln!(
                    "Insuffient funds to withdraw tid[{}]. Ignoring.",
                    tx.tid.0
                );
            }
        }
        TxType::Dispute => {
            // Unspecified behaviour when there is insufficient funds. Allow the user to enter debt when funds are disputed.
            if let Some(previous_tx) = client_entry.history.remove(&tx.tid) {
                client_entry.held += previous_tx.amount;
                client_entry.available -= previous_tx.amount;
                client_entry.disputed.insert(tx.tid, previous_tx);
            } else {
                eprintln!(
                    "Detected dispute referencing unknown previous transaction tid[{}]. Ignoring.",
                    tx.tid.0
                );
            }
        }
        TxType::Resolve => {
            if let Some(previous_tx) = client_entry.disputed.remove(&tx.tid) {
                client_entry.held -= previous_tx.amount;
                client_entry.available += previous_tx.amount;
                client_entry.history.insert(tx.tid, previous_tx);
            } else {
                eprintln!(
                    "Detected resolve referencing unknown disputed transaction tid[{}]. Ignoring.",
                    tx.tid.0
                );
            }
        }
        TxType::ChargeBack => {
            if let Some(previous_tx) = client_entry.disputed.remove(&tx.tid) {
                client_entry.held -= previous_tx.amount;
                client_entry.history.insert(tx.tid, previous_tx);
                client_entry.locked = true;
            } else {
                eprintln!("Detected chargeback referencing unknown disputed transaction tid[{}]. Ignoring.", tx.tid.0);
            }
        }
    }

    client_entry.history.insert(tx.tid, tx);
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err(BasicError::new("First and only argument is required but missing. This must specify a path to the input csv file."));
    }

    let path: &str = &args[1];
    let file = File::open(path)?;
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .has_headers(true)
        .flexible(true)
        .from_reader(file);

    let mut app_state = AppState::default();
    for row in reader.deserialize::<InputTx>() {
        if let Err(err) = row {
            eprintln!("Failed to deserialize row, skipping [{}]", err);
            break;
        }
        let tx = Tx::from(row?);
        execute_transaction(&mut app_state, tx);
    }

    println!("client,available,held,total,locked");
    for (cid, user) in app_state.clients {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(vec![]);
        writer.serialize(ClientOutputState::from(user, cid))?;
        let serialized = String::from_utf8(writer.into_inner()?)?;
        print!("{}", serialized);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: Could do more tests for scenarios including more users, and for more complicated
    // transaction chains but this should be good enough to show a pattern

    #[test]
    fn basic_deposit() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        assert_eq!(app_state.clients.len(), 1);
        assert_eq!(
            app_state.clients.entry(ClientId(1)).or_default().available,
            Currency::from_num(1.0)
        );
    }

    #[test]
    fn basic_deposit_multi_user() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 2, 1, Currency::from_num(1.0)),
        );
        assert_eq!(app_state.clients.len(), 2);
        assert_eq!(
            app_state.clients.entry(ClientId(1)).or_default().available,
            Currency::from_num(1.0)
        );
        assert_eq!(
            app_state.clients.entry(ClientId(2)).or_default().available,
            Currency::from_num(1.0)
        );
    }

    #[test]
    fn basic_withdrawal() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Withdrawal, 1, 2, Currency::from_num(0.5)),
        );
        assert_eq!(app_state.clients.len(), 1);
        assert_eq!(
            app_state.clients.entry(ClientId(1)).or_default().available,
            Currency::from_num(0.5)
        );
    }

    #[test]
    fn dispute_happy_path() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Dispute, 1, 1, Currency::default()),
        );
        assert_eq!(app_state.clients.len(), 1);
        let client_state = app_state.clients.entry(ClientId(1)).or_default();
        assert_eq!(client_state.available, Currency::from_num(0.0));
        assert_eq!(client_state.held, Currency::from_num(1.0));
        assert_eq!(client_state.locked, false);
    }

    #[test]
    fn dispute_txid_doesnt_exist() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Dispute, 1, 0, Currency::default()),
        );
        assert_eq!(app_state.clients.len(), 1);
        let client_state = app_state.clients.entry(ClientId(1)).or_default();
        assert_eq!(client_state.available, Currency::from_num(1.0));
        assert_eq!(client_state.held, Currency::from_num(0.0));
        assert_eq!(client_state.locked, false);
    }

    #[test]
    fn resolve_happy_path() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Dispute, 1, 1, Currency::default()),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Resolve, 1, 1, Currency::default()),
        );
        assert_eq!(app_state.clients.len(), 1);
        let client_state = app_state.clients.entry(ClientId(1)).or_default();
        assert_eq!(client_state.available, Currency::from_num(1.0));
        assert_eq!(client_state.held, Currency::from_num(0.0));
        assert_eq!(client_state.locked, false);
    }

    #[test]
    fn resolve_txid_doesnt_exist() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Dispute, 1, 1, Currency::default()),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Resolve, 1, 0, Currency::default()),
        );
        assert_eq!(app_state.clients.len(), 1);
        let client_state = app_state.clients.entry(ClientId(1)).or_default();
        assert_eq!(client_state.available, Currency::from_num(0.0));
        assert_eq!(client_state.held, Currency::from_num(1.0));
        assert_eq!(client_state.locked, false);
    }

    #[test]
    fn chargeback_happy_path() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Dispute, 1, 1, Currency::default()),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::ChargeBack, 1, 1, Currency::default()),
        );
        assert_eq!(app_state.clients.len(), 1);
        let client_state = app_state.clients.entry(ClientId(1)).or_default();
        assert_eq!(client_state.available, Currency::from_num(0.0));
        assert_eq!(client_state.held, Currency::from_num(0.0));
        assert_eq!(client_state.locked, true);
    }

    #[test]
    fn chargeback_txid_doesnt_exist() {
        let mut app_state = AppState::default();
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Deposit, 1, 1, Currency::from_num(1.0)),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::Dispute, 1, 1, Currency::default()),
        );
        execute_transaction(
            &mut app_state,
            Tx::new(TxType::ChargeBack, 1, 0, Currency::default()),
        );
        assert_eq!(app_state.clients.len(), 1);
        let client_state = app_state.clients.entry(ClientId(1)).or_default();
        assert_eq!(client_state.available, Currency::from_num(0.0));
        assert_eq!(client_state.held, Currency::from_num(1.0));
        assert_eq!(client_state.locked, false);
    }
}
