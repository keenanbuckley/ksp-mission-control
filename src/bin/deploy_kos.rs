use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use ksp_mission_control::config;

const SRC_DIR: &str = "kos/scripts";
const CONFIG_PATH: &str = ".kos.toml";

fn main() -> Result<()> {
    let dest = resolve_dest()?;
    let src = Path::new(SRC_DIR);
    if !src.is_dir() {
        return Err(anyhow!("source dir {} does not exist", src.display()));
    }
    let mut count = 0;
    copy_recursive(src, src, &dest, &mut count)?;
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
    if let Some(cfg) = config::read(Path::new(CONFIG_PATH))? {
        return Ok(PathBuf::from(cfg.script_dir));
    }
    Err(anyhow!(
        "no script_dir configured. Set one of: --path <dir>, KSP_SCRIPT_DIR env var, or script_dir in {CONFIG_PATH}",
    ))
}

fn copy_recursive(root: &Path, current: &Path, dest_root: &Path, count: &mut u32) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("read_dir {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_recursive(root, &path, dest_root, count)?;
        } else if file_type.is_file() && path.extension().is_some_and(|e| e == "ks") {
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("strip prefix {}", root.display()))?;
            let dest = dest_root.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create_dir_all {}", parent.display()))?;
            }
            fs::copy(&path, &dest)
                .with_context(|| format!("copy {} -> {}", path.display(), dest.display()))?;
            *count += 1;
            println!("  {} -> {}", rel.display(), dest.display());
        }
    }
    Ok(())
}
