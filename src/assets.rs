use include_dir::{include_dir, Dir};

pub static STATIC_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/static");
pub static KOS_SCRIPTS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/kos/scripts");
