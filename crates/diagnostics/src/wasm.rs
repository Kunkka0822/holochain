use crate::display::dump_kv;
use holochain_sqlite::{db, env::DbWrite, prelude::*};

pub async fn dump_wasm_state(env: DbWrite) -> anyhow::Result<()> {
    use db::*;
    let mut g = env.guard();
    let r = g.reader()?;

    dump_kv(&r, "wasm", env.get_table(&WASM)?)?;
    dump_kv(&r, "dna defs", env.get_table(&DNA_DEF)?)?;
    dump_kv(&r, "entry defs", env.get_table(&ENTRY_DEF)?)?;

    Ok(())
}
