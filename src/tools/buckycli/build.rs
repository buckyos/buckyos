use anyhow::Result;
use std::env;
use vergen::EmitBuilder;

pub fn main() -> Result<()> {
    // NOTE: This will output everything, and requires all features enabled.
    // NOTE: See the EmitBuilder documentation for configuration options.
    let is_ci = env::var("CI").map_or(false, |v| v == "true");
    if is_ci {
        EmitBuilder::builder()
            .all_build()
            .all_cargo()
            .all_git()
            .all_rustc()
            .all_sysinfo()
            .emit()?;
    }
    Ok(())
}
