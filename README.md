# honker (Rust)

Ergonomic Rust binding for [Honker](https://honker.dev). Durable queues, streams, pub/sub, and cron scheduler on SQLite. One file, zero servers.

## Install

```toml
[dependencies]
honker = "0.1"
serde_json = "1"
```

No separate extension download. This crate statically links `honker-core` and registers every `honker_*` SQL function on the connection it opens.

## Quick start

```rust
use honker::{Database, EnqueueOpts, QueueOpts};
use serde_json::json;

fn main() -> honker::Result<()> {
    let db = Database::open("app.db")?;
    let q = db.queue("emails", QueueOpts::default());

    q.enqueue(&json!({"to": "alice@example.com"}), EnqueueOpts::default())?;

    if let Some(job) = q.claim_one("worker-1")? {
        let body: serde_json::Value = job.payload_as()?;
        // send_email(body) ...
        job.ack()?;
    }
    Ok(())
}
```

## Atomic enqueue with business writes

The killer feature of a SQLite-native queue. Open a transaction, do your business INSERT, enqueue with `enqueue_tx`, commit. Rollback drops both.

```rust
let tx = db.transaction()?;
tx.execute(
    "INSERT INTO orders (user_id, total) VALUES (?1, ?2)",
    rusqlite::params![42, 9900],
)?;
q.enqueue_tx(&tx, &json!({"order_id": 42}), EnqueueOpts::default())?;
tx.commit()?;
```

## WAL-waking workers

`claim_waker` blocks until the next job lands, driven by the update watcher. No polling.

```rust
let waker = q.claim_waker();
loop {
    let Some(job) = waker.next("worker-1")? else { break; };
    // handle job ...
    job.ack()?;
}
```

## Streams (durable pub/sub)

Persistent event log with per-consumer offsets. Consumers resume after restart.

```rust
let s = db.stream("orders");
s.publish(&json!({"id": 42}))?;

for event in s.subscribe("email-worker")? {
    let event = event?;
    // event.payload_as::<MyType>()? ...
}
```

## Ephemeral pub/sub

`pg_notify`-style. Fire-and-forget, sub-2ms cross-process wake.

```rust
db.notify("orders", &json!({"id": 42}))?;

let mut sub = db.listen("orders")?;
while let Some(notif) = sub.recv() {
    let n = notif?;
    println!("{}: {}", n.channel, n.payload);
}
```

## Scheduler

Cron-style periodic tasks with leader election. Multiple processes can call `run()`; only the lock holder fires.

```rust
use honker::ScheduledTask;
use std::sync::{atomic::AtomicBool, Arc};

let sched = db.scheduler();
sched.add(ScheduledTask {
    name: "nightly".into(),
    queue: "backups".into(),
    cron: "0 3 * * *".into(),
    payload: json!({"target": "s3"}),
    priority: 0,
    expires_s: Some(3600),
})?;

let stop = Arc::new(AtomicBool::new(false));
sched.run(stop, "host-a")?;
```

## Advisory locks

RAII: the lock releases when `Lock` drops. Use `heartbeat` to extend the TTL for long-held locks.

```rust
if let Some(lock) = db.try_lock("migrations", "host-a", 60)? {
    // exclusive work ...
    lock.heartbeat(60)?;  // extend TTL
    // lock releases on drop, or call lock.release()
}
```

## Rate limiting

Fixed-window rate limit, backed by the same file.

```rust
if db.try_rate_limit("api:user:123", 100, 60)? {
    // allowed: up to 100 per 60s
}
```

## Job results

Persist a return value keyed by job id, retrievable by the caller.

```rust
db.save_result(job.id, r#"{"sent_at": 1234}"#, 3600)?;

// Later, possibly in another process:
if let Some(value) = db.get_result(job.id)? {
    // ...
}
```

## Threading

`Database` is cheap to clone (internally `Arc<Mutex<Connection>>`). Clone it across threads; every operation serializes through the shared connection. Open a second `Database` if you need a parallel reader.

`Transaction` pins the connection mutex for its lifetime. Use `*_tx` methods (`enqueue_tx`, `publish_tx`, `save_offset_tx`, `notify_tx`) to run operations inside the transaction. Calling non-tx methods on the same thread while a transaction is open deadlocks.

## Escape hatch

If you need to run custom SQL or join across `_honker_live` and your business tables:

```rust
db.with_conn(|c| {
    let n: i64 = c.query_row(
        "SELECT COUNT(*) FROM _honker_live WHERE queue = ?",
        ["emails"],
        |r| r.get(0),
    ).unwrap();
    n
});
```

## Testing

```bash
cd packages/honker-rs
cargo test
```

## Relationship to other crates

- `honker-core` is the low-level Rust crate every binding (this one, the Python PyO3 bridge, the Node napi-rs bridge, the SQLite loadable extension) shares. Import it directly only if you're writing another language binding.
- `honker` is this crate: the idiomatic Rust surface for applications.
- `honker-extension` is the loadable `.dylib`/`.so` for mixing with the `sqlite3` CLI or other-language clients that already have their own SQLite connection.
