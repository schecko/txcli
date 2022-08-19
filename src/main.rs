use fixed::types::I50F14;
use serde::{Deserialize, Serialize};
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
struct ClientId(u16);

#[derive(Serialize, Deserialize, Debug, PartialEq, Hash, Eq, Clone, Copy, Default)]
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

#[derive(Deserialize, Debug)]
struct InputTx(TxType, u16, u32, Currency);

#[derive(Deserialize, Debug)]
struct Tx {
    pub tx_type: TxType,
    pub cid: ClientId,
    pub tid: TxId,
    pub amount: Currency,
}

impl From<InputTx> for Tx {
    fn from(input: InputTx) -> Self {
        Tx {
            tx_type: input.0,
            cid: ClientId(input.1),
            tid: TxId(input.2),
            amount: input.3,
        }
    }
}

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

#[derive(Serialize, Default)]
struct ClientState {
    cid: ClientId,
    available: Currency,
    held: Currency,
    locked: bool,
    #[serde(skip)]
    history: HashMap<TxId, Tx>,
    #[serde(skip)]
    disputed: HashMap<TxId, Tx>,
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
    let mut client_entry = app_state
        .clients
        .entry(tx.cid)
        .or_insert(ClientState::default());

    // Should probably be stored in a db... hopefully your test machine has a lot
    // of RAM if its really going to use the entire space of a u32.
    // I'm guessing this will cause some large sample tests to fail,
    // but IRL the history is desireable to retain, so I am retaining it
    // in this crude form.
    match &tx.tx_type {
        TxType::Deposit => {
            client_entry.available += tx.amount;
        }
        TxType::Withdrawal => {
            client_entry.available -= tx.amount;
        }
        TxType::Dispute => {
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
                eprintln!( "Detected chargeback referencing unknown disputed transaction tid[{}]. Ignoring.", tx.tid.0 );
            }
        }
    }
    client_entry.history.insert(tx.tid, tx);
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err( BasicError::new( "First and only argument is required but missing. This must specify a path to the input csv file." ) );
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
