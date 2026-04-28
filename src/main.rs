//! Minimal repro: a workflow that exits after `ctx.run` returns but
//! before the workflow function returns cannot be resumed.
//! See README.md.

use resonate::prelude::{Resonate, ResonateConfig, Result};

#[resonate::function]
async fn step(value: String) -> Result<String> {
    Ok(format!("step({value})"))
}

#[resonate::function]
async fn workflow(ctx: &resonate::prelude::Context, crash: bool) -> Result<String> {
    let s: String = ctx.run(step, "first".to_string()).await?;
    if crash {
        eprintln!("[CRASH] exit(137) after ctx.run, before workflow returns");
        std::process::exit(137);
    }
    Ok(format!("workflow({s})"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let crash = match mode.as_str() {
        "crash" => true,
        "replay" => false,
        _ => {
            eprintln!("usage: cargo run -- <crash | replay>");
            std::process::exit(2);
        }
    };

    let r = Resonate::new(ResonateConfig {
        url: Some("http://localhost:8001".into()),
        ..Default::default()
    });
    r.register(step)?;
    r.register(workflow)?;

    println!("[{mode}] resonate.run(\"repro\", crash={crash})");
    let result: String = r.run("repro", workflow, crash).await?;
    println!("[{mode}] OK: {result}");

    r.stop().await?;
    Ok(())
}
