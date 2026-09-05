//! Install/check the native local controller in an explicitly selected private directory.
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(std::env::args_os().nth(1).ok_or("pass an existing private installation directory")?);
    let status = silicon_browser::setup::ensure_runner(&directory, |event| eprintln!("{event:?}"))?;
    println!("ready: {}", status.ready);
    Ok(())
}
