
use fixed::types::I50F14;
use std::collections::HashSet;
use serde::{ Serialize, Deserialize };

type Currency = I50F14;

#[derive(Serialize, Deserialize)]
struct ClientId(u16);

#[derive(Serialize, Deserialize)]
struct TxId(u32);

#[derive(Deserialize)]
enum Transaction
{
    Deposit(ClientId, TxId, Currency),
    Withdrawal(ClientId, TxId, Currency),
    Dispute(ClientId, TxId),
    Resolve(ClientId, TxId),
    ChargeBack(ClientId, TxId),
}

#[derive(Serialize)]
struct ClientState
{
    cid: ClientId,
    available: Currency,
    held: Currency,
    locked: bool,
}

fn main() 
{
    let mut clients = HashSet::<ClientState>::new();
    let mut reader = csv::Reader::from_reader(std::io::stdin());
    for tx in reader.records() 
    {
        println!( "{:?}", tx.unwrap() );
    }
    println!("Hello, world!");
}
