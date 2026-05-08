use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KosConfig {
    pub script_dir: String,
}

pub fn read(path: &Path) -> Result<Option<KosConfig>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(
            toml::from_str(&s).with_context(|| format!("parse {}", path.display()))?,
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

pub fn write(path: &Path, cfg: &KosConfig) -> Result<()> {
    let body = toml::to_string(cfg).context("serialize KosConfig")?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, body).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

pub fn bootstrap_if_missing(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Ok(());
    }

    print!("Enter KSP scripts directory (e.g. /path/to/Ships/Script): ");
    io::stdout().flush().ok();

    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .context("read script_dir from stdin")?;
    let script_dir = line.trim().to_string();
    if script_dir.is_empty() {
        return Err(anyhow!("script_dir was empty; not writing .kos.toml"));
    }

    write(path, &KosConfig { script_dir })?;
    info!("wrote {}", path.display());
    Ok(())
}
