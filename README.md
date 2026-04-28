# Resonate workflow-resume repro

A workflow that the host process exits *during execution* — after the
SDK has called `ctx.run` and before the workflow function returns —
cannot be resumed. Calling `resonate.run(id, ...)` again with the same
RUN_ID, while the original run is still booked on the server, fails
with:

```
Error: DecodingError("missing 'promise' in response")
```

Verified against:

- `resonate-sdk` **0.3.0** (crates.io)
- `resonate` server **0.9.4** (`ghcr.io/resonatehq/resonate:v0.9.4`)

## Reproduce

One terminal — start a fresh server:

```sh
docker run --rm -p 8001:8001 ghcr.io/resonatehq/resonate:v0.9.4 \
  serve --server-bind 0.0.0.0
```

Another terminal:

```sh
cargo run -- crash    # workflow exits 137 after ctx.run, before returning
cargo run -- replay   # same RUN_ID; expected to resume and print "workflow(step(first))"
```

### Expected

```
[replay] OK: workflow(step(first))
```

### Observed

```
[replay] resonate.run("repro", crash=false)
Error: DecodingError("missing 'promise' in response")
```

The decoding error is downstream of a 409 from the server.

## Server-log evidence

The server's view of the two runs (irrelevant lines elided):

```
# run 1 — crash
Promise created   promise_id=repro.0   state=pending      ← child promise for the ctx.run leaf
Promise settled   promise_id=repro.0   state=resolved     ← leaf finished, correctly resolved
[host process exit(137); workflow function never returned]

# run 2 — replay (immediately after)
Received request  kind=task.create  corr_id=sr-…
Request rejected  kind=task.create  status=409  elapsed_ms=0
```

After the crash, the workflow's task is still booked on the server
(versioned and cycled by `task.acquire` / `Task released back to
pending` until its TTL expires). The replay's fresh `task.create`
collides with that booking and gets 409. The SDK then surfaces the
409 body to the caller as `DecodingError("missing 'promise' in
response")`.

## Why this matters

Durable-execution's headline guarantee is "if the worker dies
mid-flow, replay with the same id picks up where it left off." On
this SDK + server pair, an immediate replay (within the task TTL
window) is rejected before it can reach the cached step.

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

Same symptom on the latest of each. Either I missed the right
release notes, or the gap is somewhere else entirely.

## Files

- `Cargo.toml` — three deps: `resonate-sdk = "0.3.0"`, `tokio`, `serde`.
- `src/main.rs` — ~45 lines. One leaf, one workflow with one `ctx.run`,
  a `crash`/`replay` CLI driver.

The workflow body in full:

```rust
#[resonate::function]
async fn workflow(ctx: &resonate::prelude::Context, crash: bool) -> Result<String> {
    let s: String = ctx.run(step, "first".to_string()).await?;
    if crash {
        eprintln!("[CRASH] exit(137) after ctx.run, before workflow returns");
        std::process::exit(137);
    }
    Ok(format!("workflow({s})"))
}
```

`step` is pure — `Ok(format!("step({value})"))`. No I/O, no app
logic. The bug is entirely in the SDK ↔ server task lifecycle around
a partially-executed workflow.
