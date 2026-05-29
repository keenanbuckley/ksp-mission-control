use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use include_dir::Dir;
use ksp_mission_control::{assets::KOS_SCRIPTS, config};

const CONFIG_PATH: &str = ".kos.toml";

fn main() -> Result<()> {
    let dest = resolve_dest()?;
    let mut count = 0;
    walk(&KOS_SCRIPTS, &dest, &mut count)?;
    println!("deployed {count} script(s) to {}", dest.display());
    Ok(())
}

fn resolve_dest() -> Result<PathBuf> {
    let mut args = env::args().skip(1);
    if let Some(arg) = args.next() {
        if arg != "--path" {
            return Err(anyhow!("unrecognized argument: {arg}"));
        }
        let v = args
            .next()
            .ok_or_else(|| anyhow!("--path requires a directory argument"))?;
        return Ok(PathBuf::from(v));
    }
    if let Ok(v) = env::var("KSP_SCRIPT_DIR") {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    let cfg_path = Path::new(CONFIG_PATH);
    config::bootstrap_if_missing(cfg_path)?;
    if let Some(cfg) = config::read(cfg_path)? {
        return Ok(PathBuf::from(cfg.script_dir));
    }
    Err(anyhow!(
        "no script_dir configured. Set one of: --path <dir>, KSP_SCRIPT_DIR env var, or script_dir in {CONFIG_PATH}",
    ))
}

fn walk(current: &Dir<'static>, dest_root: &Path, count: &mut u32) -> Result<()> {
    for file in current.files() {
        let rel = file.path();
        if rel.extension().is_some_and(|e| e == "ks") {
            let dest = dest_root.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create_dir_all {}", parent.display()))?;
            }
            fs::write(&dest, file.contents())
                .with_context(|| format!("write {}", dest.display()))?;
            *count += 1;
            println!("  {} -> {}", rel.display(), dest.display());
        }
    }
    for dir in current.dirs() {
        walk(dir, dest_root, count)?;
    }
    Ok(())
}
