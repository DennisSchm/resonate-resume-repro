//! Minimal repro: a `#[resonate::function]` whose worker exits before
//! it returns cannot be re-issued with the same id. See README.md.

use resonate::prelude::{Resonate, ResonateConfig, Result};

#[resonate::function]
async fn task(crash: bool) -> Result<String> {
    if crash {
        eprintln!("[CRASH] exit(137) before task returns");
        std::process::exit(137);
    }
    Ok("done".into())
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
    r.register(task)?;

    let result: String = r.run("repro", task, crash).await?;
    println!("[{mode}] OK: {result}");

    r.stop().await?;
    Ok(())
}
