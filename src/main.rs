//! Minimal repro: a workflow with two `ctx.run` steps cannot be
//! resumed when the host process exits between step 1 and step 2.
//!
//! Run order:
//!   1. `cargo run -- crash`   — workflow exits 137 after `ctx.run` #1.
//!   2. `cargo run -- replay`  — same RUN_ID; expected to resume at
//!                                 step 2 and return "workflow done: …".
//!                                 Observed: server returns 409 to the
//!                                 SDK's `task.create`, the SDK then
//!                                 errors with `decoding error: missing
//!                                 'promise' in response`.
//!
//! See README.md for the full description and a server-log walkthrough.

use std::env;
use std::process;

// `Context` is referenced in the workflow signature below, but the
// `#[resonate::function]` macro rewrites the body to use a fully-
// qualified path internally, so the compiler reads the import as
// unused. Keep it imported for human readability.
#[allow(unused_imports)]
use resonate::prelude::Context;
use resonate::prelude::{Resonate, ResonateConfig, Result};
use serde::{Deserialize, Serialize};

const SERVER_URL: &str = "http://localhost:8001";
const RUN_ID: &str = "minimal-resume-repro";

#[derive(Serialize, Deserialize)]
struct LeafArgs {
    label: String,
}

#[derive(Serialize, Deserialize)]
struct WorkflowArgs {
    crash_between_steps: bool,
}

/// Trivial leaf #1 — pure, no I/O. Stands in for any side-effecting
/// step that has already completed and been checkpointed by the time
/// the host process dies.
#[resonate::function]
async fn leaf_a(args: LeafArgs) -> Result<String> {
    Ok(format!("a:{}", args.label))
}

/// Trivial leaf #2 — pure, no I/O. Stands in for the step that should
/// run on the resume attempt.
#[resonate::function]
async fn leaf_b(args: LeafArgs) -> Result<String> {
    Ok(format!("b:{}", args.label))
}

/// Workflow with two `ctx.run` steps and a controlled crash point in
/// between. The bug surfaces when the first invocation runs with
/// `crash_between_steps = true` and a second invocation with the same
/// RUN_ID and `crash_between_steps = false` is then issued — Resonate
/// is supposed to replay step 1 from cache and run step 2 fresh, but
/// rejects the second `task.create` with 409.
#[resonate::function]
async fn workflow(ctx: &Context, args: WorkflowArgs) -> Result<String> {
    let _step1: String = ctx
        .run(leaf_a, LeafArgs { label: "step1".into() })
        .await?;

    if args.crash_between_steps {
        eprintln!("[CRASH] exit(137) between ctx.run #1 and ctx.run #2");
        process::exit(137);
    }

    let step2: String = ctx
        .run(leaf_b, LeafArgs { label: "step2".into() })
        .await?;

    Ok(format!("workflow done: {step2}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = env::args().nth(1).unwrap_or_default();
    let crash = match mode.as_str() {
        "crash" => true,
        "replay" => false,
        _ => {
            eprintln!("usage: cargo run -- <crash | replay>");
            process::exit(2);
        }
    };

    let r = Resonate::new(ResonateConfig {
        url: Some(SERVER_URL.into()),
        ..Default::default()
    });
    r.register(leaf_a)?;
    r.register(leaf_b)?;
    r.register(workflow)?;

    println!("[{mode}] resonate.run({RUN_ID:?}, crash_between_steps={crash})");
    let result: String = r
        .run(
            RUN_ID,
            workflow,
            WorkflowArgs {
                crash_between_steps: crash,
            },
        )
        .await?;
    println!("[{mode}] OK: {result}");

    r.stop().await?;
    Ok(())
}
