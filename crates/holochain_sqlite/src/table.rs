//! Functionality for safely accessing LMDB database references.

use crate::prelude::Writer;
use crate::{db::DbKind, exports::IntegerTable, prelude::Readable};
use crate::{
    error::DatabaseResult,
    exports::{MultiTable, SingleTable},
};
use derive_more::Display;
use rusqlite::{types::Value, *};
use std::path::Path;

/// Enumeration of all databases needed by Holochain
#[derive(Clone, Debug, Hash, PartialEq, Eq, Display)]
pub enum TableName {
    /// Vault database: KV store of chain entries, keyed by address
    ElementVaultPublicEntries,
    /// Vault database: KV store of chain entries, keyed by address
    ElementVaultPrivateEntries,
    /// Vault database: KV store of chain headers, keyed by address
    ElementVaultHeaders,
    /// Vault database: KVV store of chain metadata, storing relationships
    MetaVaultSys,
    /// Vault database: Kv store of links
    MetaVaultLinks,
    /// Vault database: Kv store of entry dht status
    MetaVaultMisc,
    /// int KV store storing the sequence of committed headers,
    /// most notably allowing access to the chain head
    ChainSequence,
    /// Cache database: KV store of chain entries, keyed by address
    ElementCacheEntries,
    /// Cache database: KV store of chain headers, keyed by address
    ElementCacheHeaders,
    /// Cache database: KVV store of chain metadata, storing relationships
    MetaCacheSys,
    /// Cache database: Kv store of links
    MetaCacheLinks,
    /// Vault database: Kv store of entry dht status
    MetaCacheStatus,
    /// database which stores a single key-value pair, encoding the
    /// mutable state for the entire Conductor
    ConductorState,
    /// database that stores wasm bytecode
    Wasm,
    /// database to store the [DnaDef]
    DnaDef,
    /// database to store the [EntryDef] Kvv store
    EntryDef,
    /// Authored [DhtOp]s KV store
    AuthoredDhtOps,
    /// Integrated [DhtOp]s KV store
    IntegratedDhtOps,
    /// Integration Queue of [DhtOp]s KV store where key is [DhtOpHash]
    IntegrationLimbo,
    /// Place for [DhtOp]s waiting to be validated to hang out. KV store where key is a [DhtOpHash]
    ValidationLimbo,
    /// KVV store to accumulate validation receipts for a published EntryHash
    ValidationReceipts,
    /// Single store for all known agents on the network
    Agent,
}

impl ToSql for TableName {
    fn to_sql(&self) -> Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::Owned(
            format!("{}", self).into(),
        ))
    }
}

fn initialize_table(conn: &mut Connection, name: TableName) -> DatabaseResult<()> {
    let table_name = format!("{}", name);
    let index_name = format!("{}_idx", table_name);

    // create table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ?1 (
            key       BLOB PRIMARY KEY,
            val       BLOB NOT NULL
        );",
        &[table_name.clone()],
    )?;

    // create index
    conn.execute(
        "CREATE INDEX IF NOT EXISTS ?1 ON ?2 ( key );",
        &[index_name, table_name],
    )?;
    Ok(())
}

pub(super) fn initialize_database(conn: &mut Connection, kind: &DbKind) -> DatabaseResult<()> {
    match kind {
        DbKind::Cell(_) => {
            initialize_table(conn, TableName::ElementVaultPublicEntries)?;
            initialize_table(conn, TableName::ElementVaultPrivateEntries)?;
            initialize_table(conn, TableName::ElementVaultHeaders)?;
            initialize_table(conn, TableName::MetaVaultSys)?;
            initialize_table(conn, TableName::MetaVaultLinks)?;
            initialize_table(conn, TableName::MetaVaultMisc)?;
            initialize_table(conn, TableName::ChainSequence)?;
            initialize_table(conn, TableName::ElementCacheEntries)?;
            initialize_table(conn, TableName::ElementCacheHeaders)?;
            initialize_table(conn, TableName::MetaCacheSys)?;
            initialize_table(conn, TableName::MetaCacheLinks)?;
            initialize_table(conn, TableName::MetaCacheStatus)?;
            initialize_table(conn, TableName::AuthoredDhtOps)?;
            initialize_table(conn, TableName::IntegratedDhtOps)?;
            initialize_table(conn, TableName::IntegrationLimbo)?;
            initialize_table(conn, TableName::ValidationLimbo)?;
            initialize_table(conn, TableName::ValidationReceipts)?;
        }
        DbKind::Conductor => {
            initialize_table(conn, TableName::ConductorState)?;
        }
        DbKind::Wasm => {
            initialize_table(conn, TableName::Wasm)?;
            initialize_table(conn, TableName::DnaDef)?;
            initialize_table(conn, TableName::EntryDef)?;
        }
        DbKind::P2p => {
            initialize_table(conn, TableName::Agent)?;
            // @todo health metrics for the space
            // register_db(env, um, &*HEALTH)?;
        }
    }
    Ok(())
}

/// TODO
#[deprecated = "sqlite: placeholder"]
pub trait GetTable {
    /// Placeholder
    fn get_table(&self, name: TableName) -> DatabaseResult<Table> {
        Ok(Table { name })
    }

    /// Placeholder
    #[deprecated = "use get_table"]
    fn get_table_i(&self, name: TableName) -> DatabaseResult<Table> {
        self.get_table(name)
    }

    /// Placeholder
    #[deprecated = "use get_table"]
    fn get_table_m(&self, name: TableName) -> DatabaseResult<Table> {
        self.get_table(name)
    }
}

/// A reference to a SQLite table.
/// This patten only exists as part of the naive LMDB refactor.
#[deprecated = "lmdb: naive"]
#[derive(Clone, Debug)]
pub struct Table {
    name: TableName,
}

impl Table {
    pub fn name(&self) -> &TableName {
        &self.name
    }

    /// TODO: would be amazing if this could return a ValueRef instead.
    ///       but I don't think it's possible. Could use a macro instead...
    pub fn get<R: Readable, K: AsRef<[u8]>>(
        &self,
        reader: &mut R,
        k: K,
    ) -> DatabaseResult<Option<Value>> {
        Ok(reader.get(self, k)?)
    }

    /// This handles the fact that getting from an rkv::MultiTable returns
    /// multiple results
    #[deprecated = "unneeded in the context of SQL"]
    pub fn get_m<R: Readable, K: ToSql>(
        &self,
        reader: &mut R,
        k: K,
    ) -> DatabaseResult<impl Iterator<Item = DatabaseResult<(K, Option<Value>)>>> {
        todo!();
        Ok(std::iter::empty())
    }

    pub fn put<K: ToSql>(&self, writer: &mut Writer, k: K, v: &Value) -> DatabaseResult<()> {
        todo!()
    }

    #[deprecated = "unneeded in the context of SQL"]
    pub fn put_with_flags<K: ToSql>(
        &self,
        writer: &mut Writer,
        k: K,
        v: &Value,
        flags: (),
    ) -> DatabaseResult<()> {
        todo!()
    }

    pub fn delete<K: ToSql>(&self, writer: &mut Writer, k: K) -> DatabaseResult<()> {
        todo!()
    }

    pub fn delete_all<K: ToSql>(&self, writer: &mut Writer, k: K) -> DatabaseResult<()> {
        todo!()
    }

    /// This handles the fact that deleting from an rkv::MultiTable requires
    /// passing the value to delete (deleting a particular kv pair)
    #[deprecated = "unneeded in the context of SQL"]
    pub fn delete_m<K: ToSql>(&self, writer: &mut Writer, k: K, v: &Value) -> DatabaseResult<()> {
        todo!()
    }

    #[cfg(feature = "test_utils")]
    pub fn clear(&mut self, writer: &mut Writer) -> DatabaseResult<()> {
        todo!()
    }
}

// TODO: probably remove
// /// Macros to produce `impl Iterator` over a table's keys and values.
// /// A macro is necessary to reduce boilerplate, since rusqlite's interface
// /// requires preparing a statement and querying it in separate steps.
// /// A normal function can't work with these intermediate values because the
// /// borrow checker complains about dropped values
// #[macro_export]
// macro_rules! table_iter {
//     (+ $table:ident) => {
//         let stmt = txn.prepare_cached("SELECT (key, val) FROM ?1 ASC")?;
//         stmt.query_map(params![table.name], |row| Ok((row.get(0)?, row.get(1)?)))?
//     };
//     (- $table:ident) => {
//         let stmt = txn.prepare_cached("SELECT (key, val) FROM ?1 DESC")?;
//         stmt.query_map(params![table.name], |row| Ok((row.get(0)?, row.get(1)?)))?
//     };
//     (+ $table:ident, $key:ident) => {
//         let stmt = txn.prepare_cached("SELECT (key, val) FROM ?1 WHERE key >= ?2 ASC")?;
//         stmt.query_map(params![table.name, $key], |row| {
//             Ok((row.get(0)?, row.get(1)?))
//         })?
//     };
// }

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    SqlError(#[from] rusqlite::Error),
}

pub type StoreResult<T> = Result<T, StoreError>;

impl StoreError {
    pub fn ok_if_not_found(self) -> StoreResult<()> {
        todo!("implement for rusqlite errors")
        // match self {
        //     StoreError::LmdbStoreError(err) => match err.into_inner() {
        //         rkv::StoreError::LmdbError(rkv::LmdbError::NotFound) => Ok(()),
        //         err => Err(err.into()),
        //     },
        //     err => Err(err),
        // }
    }
}
