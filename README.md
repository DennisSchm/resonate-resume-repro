# Resonate worker-crash resume repro

> **Status: fixed upstream in `resonate-sdk` 0.4.0.** This repo is kept
> as the historical reproducer that drove the fix. `Cargo.toml` is
> pinned to `0.4`; `cargo run -- crash && cargo run -- replay` against
> `ghcr.io/resonatehq/resonate:v0.9.4` now prints `[replay] OK: done`
> instead of surfacing `DecodingError`.

A `#[resonate::function]` whose worker exits before it returns cannot
be re-issued with the same id while the original run is still booked
on the server. The second `resonate.run(id, …)` call hangs briefly,
then fails with:

```
Error: DecodingError("missing 'promise' in response")
```

Originally observed against:

- `resonate-sdk` **0.3.0** (crates.io) — broken
- `resonate-sdk` **0.4.0** (crates.io) — fixed
- `resonate` server **0.9.4** (`ghcr.io/resonatehq/resonate:v0.9.4`)

## Reproduce

One terminal — fresh server every attempt (the `--rm` matters; see
note below):

```sh
docker run --rm -p 8001:8001 ghcr.io/resonatehq/resonate:v0.9.4 \
  serve --server-bind 0.0.0.0
```

Another terminal:

```sh
cargo run -- crash    # task exits 137 before returning
cargo run -- replay   # same id; expected to resume and print OK
```

### Expected

```
[replay] OK: done
```

### Observed

```
[crash]  resonate.run("repro", task, true)
[CRASH] exit(137) before task returns

[replay] resonate.run("repro", task, false)
Error: DecodingError("missing 'promise' in response")
```

> **Re-running:** restart the docker server (`Ctrl-C` then re-run the
> `docker run` command) between attempts. With `--rm` this gives a
> clean slate; without it, server-side state from prior runs cycles
> the task through `task.acquire` / `Task released back to pending`
> and the SDK occasionally re-runs the cached invocation instead of
> surfacing the decoding error. The bug is the same either way, but
> the decoding-error symptom is what's worth showing.

## Server-log evidence

```
# run 1 (crash)
Received request  kind=task.create   corr_id=…  status=200          ← worker books the task
[host process exit(137); task never marked complete]

# run 2 (replay), issued seconds later
Received request  kind=task.create   corr_id=…
Request rejected  kind=task.create   status=409   elapsed_ms=0      ← original task still booked
```

The SDK then surfaces the 409 body to the caller as
`DecodingError("missing 'promise' in response")`.

## Why this matters

Durable-execution's headline guarantee is "if the worker dies
mid-flow, replay with the same id picks up where it left off." On
this SDK + server pair, an immediate replay (within the original
task's TTL window) is rejected before it can reach a useful state.

Notably, the bug **does not require `ctx.run`** — it surfaces with a
pure leaf function that exits before returning. So it isn't a
checkpoint-resume issue; it's a task-lifecycle issue around a worker
that dropped a booked task without completing it.

Two possibilities I haven't sorted out:

1. **The SDK should call a different endpoint on replay.** `task.create`
   is a fresh-start verb; resume needs a claim-or-attach.
2. **The server should release the booking faster** when the worker
   stops heartbeating, or accept `task.create` against an
   already-booked task as a takeover signal.

I tried bumping both sides to the latest releases that mentioned
protocol/encoding fixes:

- SDK 0.1.0 → 0.3.0 ("Fix protocol version and subclient encodings")
- server 0.9.3 → 0.9.4 ("Linearizability and version-only-on-claim fixes")

Same symptom on the latest of each.

## Files

- `Cargo.toml` — three deps: `resonate-sdk = "0.3.0"`, `tokio`, `serde`.
- `src/main.rs` — **27 lines**. One leaf, one `crash`/`replay` CLI mode,
  no workflow, no `ctx.run`, no extra structs.

The whole task in full:

```rust
#[resonate::function]
async fn task(crash: bool) -> Result<String> {
    if crash {
        eprintln!("[CRASH] exit(137) before task returns");
        std::process::exit(137);
    }
    Ok("done".into())
}
```
