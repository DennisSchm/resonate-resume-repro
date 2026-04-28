# Resonate workflow-resume repro

A workflow that completes one `ctx.run` step and then crashes the host
process cannot be resumed: a second invocation with the same RUN_ID
fails with `DecodingError("missing 'promise' in response")`. The
underlying cause is server-side: the parent promise is settled to
`resolved` immediately after the first `ctx.run` leaf finishes,
**before the workflow function itself returns**, so the replay's
`task.create` is rejected with 409 against an already-resolved
promise.

Verified against:

- `resonate-sdk` **0.3.0** (crates.io)
- `resonate` server **0.9.4** (`ghcr.io/resonatehq/resonate:v0.9.4`)

Both are the latest at time of writing (28 April 2026).

## Reproduce

One terminal — start a fresh Resonate server:

```sh
docker run --rm -p 8001:8001 ghcr.io/resonatehq/resonate:v0.9.4 \
  serve --server-bind 0.0.0.0
```

Another terminal — run the workflow once, crashing it after step 1,
then run it again with the same RUN_ID:

```sh
cargo run -- crash    # exits 137 between ctx.run #1 and ctx.run #2
cargo run -- replay   # same RUN_ID; should resume at step 2
```

### Expected

The replay should observe step 1's cached outcome, run step 2, and
print:

```
[replay] OK: workflow done: b:step2
```

### Observed

```
[replay] resonate.run("minimal-resume-repro", crash_between_steps=false)
Error: DecodingError("missing 'promise' in response")
```

## Server log

What the resonate server prints during the two runs:

```
Promise created   minimal-resume-repro.0  state=pending      ← run 1, ctx.run #1 creates the workflow promise
Promise settled   minimal-resume-repro.0  state=resolved     ← still run 1, ~1 ms after creation, *before* the workflow function returns
                                                                (this is the bug — the parent promise is being resolved
                                                                from the first child's resolution rather than from the
                                                                workflow function's return value)
[host process exit(137)]
Received request  kind=task.create                           ← run 2 (replay)
Request rejected  kind=task.create  status=409               ← the promise is already resolved, so a new task can't be booked
```

The SDK's response decoder then surfaces the 409 body as
`DecodingError("missing 'promise' in response")`.

## Why this matters

The whole point of the durable-execution contract is "if the host dies
mid-workflow, replaying with the same id resumes at the first
uncheckpointed step." This repro is the smallest possible workflow
where that contract has to hold (two `ctx.run` steps with a death
between them) and it doesn't. The implication is that *any* workflow
with two or more checkpoints can't be resumed after a crash on the
SDK + server combination above.

## What we expected to fix it

I tried both:

1. Bumping the SDK from a `git`-pinned `resonate-sdk-rust-next`
   revision to the published `resonate-sdk` 0.3.0 (release notes:
   "Fix protocol version and subclient encodings"). Same failure.
2. Bumping the server from 0.9.3 to 0.9.4 (release notes:
   "Linearizability and version-only-on-claim fixes"). Same failure.

So the gap is somewhere I haven't accounted for. It looks server-side
to me — the `Promise settled` log line lands ~1 ms after `Promise
created` and the host process is still alive — but I'm guessing.

## Files

```
src/main.rs   – the workflow + the crash/replay driver, ~95 lines
Cargo.toml    – three deps: resonate-sdk 0.3.0, tokio, serde
```

The workflow itself is the simplest shape that surfaces the bug:

```rust
#[resonate::function]
async fn workflow(ctx: &Context, args: WorkflowArgs) -> Result<String> {
    let _step1: String = ctx.run(leaf_a, LeafArgs { label: "step1".into() }).await?;

    if args.crash_between_steps {
        eprintln!("[CRASH] exit(137) between ctx.run #1 and ctx.run #2");
        process::exit(137);
    }

    let step2: String = ctx.run(leaf_b, LeafArgs { label: "step2".into() }).await?;
    Ok(format!("workflow done: {step2}"))
}
```

`leaf_a` and `leaf_b` are pure — they just return a string. No real
I/O, no state, no application logic. The bug is entirely in the
SDK ↔ server interaction around the workflow promise's lifecycle.
