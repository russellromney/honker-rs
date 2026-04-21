//! Atomic business-write + enqueue in one transaction.
//!
//! honker.Queue.enqueue can be called inside a connection-level
//! transaction alongside your own INSERTs. If the transaction
//! commits, both writes land; if it rolls back, both vanish. No
//! dual-write, no outbox table.
//!
//!     cargo run --release --example atomic

use honker::{Database, EnqueueOpts, QueueOpts};
use rusqlite::params;
use serde_json::json;

fn main() -> honker::Result<()> {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_path = tmp.path().join("app.db");
    let db = Database::open(&db_path)?;

    // Plain business table — nothing honker-specific.
    db.conn().execute(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, total INTEGER)",
        [],
    )?;

    let q = db.queue("emails", QueueOpts::default());

    // Success path: order INSERT and job enqueue commit together.
    // The honker::Queue calls honker_enqueue under the hood; calling
    // q.enqueue inside a transaction on the same connection makes
    // it atomic.
    db.conn().execute("BEGIN IMMEDIATE", [])?;
    db.conn().execute(
        "INSERT INTO orders (user_id, total) VALUES (?, ?)",
        params![42, 9900],
    )?;
    q.enqueue(
        &json!({"to": "alice@example.com", "order_id": 42}),
        EnqueueOpts::default(),
    )?;
    db.conn().execute("COMMIT", [])?;

    let orders: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM orders", [], |r| r.get(0))?;
    let queued: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM _honker_live WHERE queue='emails'",
        [],
        |r| r.get(0),
    )?;
    println!("committed: {} order(s), {} job(s)", orders, queued);
    assert_eq!(orders, 1);
    assert_eq!(queued, 1);

    // Rollback path: everything disappears.
    db.conn().execute("BEGIN IMMEDIATE", [])?;
    db.conn().execute(
        "INSERT INTO orders (user_id, total) VALUES (?, ?)",
        params![43, 5000],
    )?;
    q.enqueue(
        &json!({"to": "bob@example.com", "order_id": 43}),
        EnqueueOpts::default(),
    )?;
    db.conn().execute("ROLLBACK", [])?;
    println!("rolled back: simulated payment-processing failure");

    let orders: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM orders", [], |r| r.get(0))?;
    let queued: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM _honker_live WHERE queue='emails'",
        [],
        |r| r.get(0),
    )?;
    println!("after rollback: {} order(s), {} job(s)", orders, queued);
    assert_eq!(orders, 1);
    assert_eq!(queued, 1);

    println!("atomic enqueue + rollback both work as expected");
    Ok(())
}
