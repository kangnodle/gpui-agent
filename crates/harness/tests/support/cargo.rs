#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
use std::{env,
          fs,
          io::Write};

fn main() -> anyhow::Result<()> {
    if env::args()
        .next()
        .is_some_and(|arg| std::path::Path::new(&arg).file_stem().is_some_and(|stem| stem == "cargo-real"))
    {
        std::process::exit(99);
    }
    let log = env::var("HARNESS_TEST_LOG")?;
    let index = fs::read_to_string(&log).unwrap_or_default().lines().count();
    let mut file = fs::OpenOptions::new().create(true).append(true).open(log)?;
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "cwd": env::current_dir()?,
            "args": env::args().skip(1).collect::<Vec<_>>(),
            "mise_environment": env::var("HARNESS_TEST_MISE_ENV").ok()
        })
    )?;
    if env::var("HARNESS_TEST_FAIL").ok().as_deref() == Some(&index.to_string()) {
        std::process::exit(23);
    }
    Ok(())
}
