/// A convenient composition of CasBufs, representing source chain data.
///
/// Source chain data is split into three databases: one for headers, and two
/// for public and private entries. Specifying the private_entries DB in a
/// ChainCasBuf is optional, so that if it's not supplied, the ChainCasBuf
/// will not be able to access private data. This restriction is useful when
/// using the ChainCasBuf for caching non-authored data, or for situations where
/// it is known that private entries should be protected, such as when handling
/// a get_entry request from the network.
use crate::core::state::source_chain::{
    ChainElement, ChainInvalidReason, SignedHeaderHashed, SourceChainError, SourceChainResult,
};
use header::EntryVisibility;
use holo_hash::{EntryHash, Hashed, HeaderHash};
use holochain_state::{
    buffer::{BufferedStore, CasBuf},
    db::{
        GetDb, CACHE_CHAIN_ENTRIES, CACHE_CHAIN_HEADERS, PRIMARY_CHAIN_HEADERS,
        PRIMARY_CHAIN_PRIVATE_ENTRIES, PRIMARY_CHAIN_PUBLIC_ENTRIES,
    },
    error::{DatabaseError, DatabaseResult},
    exports::SingleStore,
    prelude::{Readable, Reader, Writer},
};
use holochain_types::{
    address::{EntryAddress, HeaderAddress},
    entry::{Entry, EntryHashed},
    header,
    prelude::Signature,
    Header, HeaderHashed,
};
use tracing::*;

/// A CasBuf with Entries for values
pub type EntryCas<'env, R> = CasBuf<'env, Entry, R>;
/// A CasBuf with SignedHeaders for values
pub type HeaderCas<'env, R> = CasBuf<'env, (Header, Signature), R>;

/// The representation of a chain CAS, using two or three DB references
pub struct ChainCasBuf<'env, R: Readable = Reader<'env>> {
    public_entries: EntryCas<'env, R>,
    private_entries: Option<EntryCas<'env, R>>,
    headers: HeaderCas<'env, R>,
}

impl<'env, R: Readable> ChainCasBuf<'env, R> {
    fn new(
        reader: &'env R,
        public_entries_store: SingleStore,
        private_entries_store: Option<SingleStore>,
        headers_store: SingleStore,
    ) -> DatabaseResult<Self> {
        let private_entries = if let Some(store) = private_entries_store {
            Some(CasBuf::new(reader, store)?)
        } else {
            None
        };
        Ok(Self {
            public_entries: CasBuf::new(reader, public_entries_store)?,
            private_entries,
            headers: CasBuf::new(reader, headers_store)?,
        })
    }

    /// Create a ChainCasBuf using the source chain databases.
    /// The `allow_private` argument allows you to specify whether private
    /// entries should be readable or writeable with this reference.
    pub fn primary(reader: &'env R, dbs: &impl GetDb, allow_private: bool) -> DatabaseResult<Self> {
        let headers = dbs.get_db(&*PRIMARY_CHAIN_HEADERS)?;
        let entries = dbs.get_db(&*PRIMARY_CHAIN_PUBLIC_ENTRIES)?;
        let private_entries = if allow_private {
            Some(dbs.get_db(&*PRIMARY_CHAIN_PRIVATE_ENTRIES)?)
        } else {
            None
        };
        Self::new(reader, entries, private_entries, headers)
    }

    /// Create a ChainCasBuf using the cache databases.
    /// There is no cache for private entries, so private entries are disallowed
    pub fn cache(reader: &'env R, dbs: &impl GetDb) -> DatabaseResult<Self> {
        let entries = dbs.get_db(&*CACHE_CHAIN_ENTRIES)?;
        let headers = dbs.get_db(&*CACHE_CHAIN_HEADERS)?;
        Self::new(reader, entries, None, headers)
    }

    /// Get an entry by its address
    ///
    /// First attempt to get from the public entry DB. If not present, and
    /// private DB access is specified, attempt to get as a private entry.
    pub fn get_entry(&self, entry_address: EntryAddress) -> DatabaseResult<Option<Entry>> {
        match self.public_entries.get(&entry_address.clone().into())? {
            Some(entry) => Ok(Some(entry)),
            None => {
                if let Some(ref db) = self.private_entries {
                    db.get(&entry_address.into())
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn contains(&self, entry_address: EntryAddress) -> DatabaseResult<bool> {
        self.get_entry(entry_address).map(|e| e.is_some())
    }

    pub async fn get_header(
        &self,
        header_address: &HeaderAddress,
    ) -> DatabaseResult<Option<SignedHeaderHashed>> {
        if let Ok(Some((header, signature))) = self.headers.get(&header_address.to_owned().into()) {
            let header = fatal_db_deserialize_check!(
                "ChainCasBuf::get_header",
                header_address,
                HeaderHashed::with_data(header).await,
            );
            fatal_db_hash_check!("ChainCasBuf::get_header", header_address, header.as_hash());
            Ok(Some(SignedHeaderHashed::with_presigned(header, signature)))
        } else {
            Ok(None)
        }
    }

    /// Get the Entry out of Header if it exists.
    ///
    /// If the header contains no entry data, return None
    /// If the header contains entry data:
    /// - if it is a public entry, but the entry cannot be found, return error
    /// - if it is a private entry and cannot be found, return error
    /// - if it is a private entry but the private DB is disabled, return None
    fn get_entry_from_header(&self, header: &Header) -> SourceChainResult<Option<Entry>> {
        Ok(match header.entry_data() {
            None => None,
            Some((entry_address, entry_type)) => {
                match entry_type.visibility() {
                    // if the header references an entry and the database is
                    // available, it better have been stored!
                    EntryVisibility::Public => Some(
                        self.public_entries
                            .get(&entry_address.clone().into())?
                            .ok_or_else(|| {
                                SourceChainError::InvalidStructure(ChainInvalidReason::MissingData(
                                    entry_address.clone(),
                                ))
                            })?,
                    ),
                    EntryVisibility::Private => {
                        if let Some(ref db) = self.private_entries {
                            Some(db.get(&entry_address.clone().into())?.ok_or_else(|| {
                                SourceChainError::InvalidStructure(ChainInvalidReason::MissingData(
                                    entry_address.clone(),
                                ))
                            })?)
                        } else {
                            // If the private DB is disabled, just return None
                            None
                        }
                    }
                }
            }
        })
    }

    /// given a header address return the full chain element for that address
    pub async fn get_element(
        &self,
        header_address: &HeaderAddress,
    ) -> SourceChainResult<Option<ChainElement>> {
        if let Some(signed_header) = self.get_header(header_address).await? {
            let maybe_entry = self.get_entry_from_header(signed_header.header())?;
            Ok(Some(ChainElement::new(signed_header, maybe_entry)))
        } else {
            Ok(None)
        }
    }

    /// Puts a signed header and optional entry onto the CAS.
    /// N.B. this code assumes that the header and entry have been validated
    pub fn put(
        &mut self,
        signed_header: SignedHeaderHashed,
        maybe_entry: Option<EntryHashed>,
    ) -> DatabaseResult<()> {
        let (header, signature) = signed_header.into_inner();
        let (header, header_address) = header.into_inner();

        if let Some(entry) = maybe_entry {
            let (entry, entry_address) = entry.into_inner();
            if let Some((_, entry_type)) = header.entry_data() {
                match entry_type.visibility() {
                    EntryVisibility::Public => self.public_entries.put(entry_address.into(), entry),
                    EntryVisibility::Private => {
                        if let Some(db) = self.private_entries.as_mut() {
                            db.put(entry_address.into(), entry);
                        } else {
                            error!("Attempted ChainCasBuf::put on a private entry with a disabled private DB: {}", entry_address);
                        }
                    }
                }
            } else {
                unreachable!(
                    "Attempting to put an entry, but the header has no entry_type. Header hash: {}",
                    header_address
                );
            }
        }

        self.headers.put(header_address.into(), (header, signature));
        Ok(())
    }

    pub fn delete(&mut self, header_hash: HeaderHash, entry_hash: EntryHash) {
        self.headers.delete(header_hash.into());
        self.public_entries.delete(entry_hash.clone().into());
        if let Some(db) = self.private_entries.as_mut() {
            db.delete(entry_hash.into())
        }
    }

    pub fn headers(&self) -> &HeaderCas<'env, R> {
        &self.headers
    }

    pub fn public_entries(&self) -> &EntryCas<'env, R> {
        &self.public_entries
    }
}

impl<'env, R: Readable> BufferedStore<'env> for ChainCasBuf<'env, R> {
    type Error = DatabaseError;

    fn flush_to_txn(self, writer: &'env mut Writer) -> DatabaseResult<()> {
        self.public_entries.flush_to_txn(writer)?;
        if let Some(db) = self.private_entries {
            db.flush_to_txn(writer)?
        };
        self.headers.flush_to_txn(writer)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::ChainCasBuf;
    use crate::test_utils::fake_unique_element;
    use holo_hash::*;
    use holochain_keystore::test_keystore::spawn_test_keystore;
    use holochain_keystore::AgentPubKeyExt;
    use holochain_state::{prelude::*, test_utils::test_cell_env};
    use holochain_types::header::EntryVisibility;

    #[tokio::test(threaded_scheduler)]
    async fn can_write_private_entry_when_enabled() -> anyhow::Result<()> {
        let keystore = spawn_test_keystore(Vec::new()).await?;
        let arc = test_cell_env();
        let env = arc.guard().await;

        let agent_key = AgentPubKey::new_from_pure_entropy(&keystore).await?;
        let (header_pub, entry_pub) =
            fake_unique_element(&keystore, agent_key.clone(), EntryVisibility::Public).await?;
        let (header_priv, entry_priv) =
            fake_unique_element(&keystore, agent_key.clone(), EntryVisibility::Private).await?;

        // write one public-entry header and one private-entry header
        env.with_commit(|txn| {
            let reader = env.reader()?;
            let mut store = ChainCasBuf::primary(&reader, &env, true)?;
            store.put(header_pub, Some(entry_pub.clone()))?;
            store.put(header_priv, Some(entry_priv.clone()))?;
            store.flush_to_txn(txn)
        })?;

        // Can retrieve both entries when private entries are enabled
        {
            let reader = env.reader()?;
            let store = ChainCasBuf::primary(&reader, &env, true)?;
            assert_eq!(
                store.get_entry(entry_pub.as_hash().clone()),
                Ok(Some(entry_pub.as_content().clone()))
            );
            assert_eq!(
                store.get_entry(entry_priv.as_hash().clone()),
                Ok(Some(entry_priv.as_content().clone()))
            );
        }

        // Cannot retrieve private entry when disabled
        {
            let reader = env.reader()?;
            let store = ChainCasBuf::primary(&reader, &env, false)?;
            assert_eq!(
                store.get_entry(entry_pub.as_hash().clone()),
                Ok(Some(entry_pub.as_content().clone()))
            );
            assert_eq!(store.get_entry(entry_priv.as_hash().clone()), Ok(None));
        }

        Ok(())
    }

    #[tokio::test(threaded_scheduler)]
    async fn cannot_write_private_entry_when_disabled() -> anyhow::Result<()> {
        let keystore = spawn_test_keystore(Vec::new()).await?;
        let arc = test_cell_env();
        let env = arc.guard().await;

        let agent_key = AgentPubKey::new_from_pure_entropy(&keystore).await?;
        let (header_pub, entry_pub) =
            fake_unique_element(&keystore, agent_key.clone(), EntryVisibility::Public).await?;
        let (header_priv, entry_priv) =
            fake_unique_element(&keystore, agent_key.clone(), EntryVisibility::Private).await?;

        // write one public-entry header and one private-entry header (which will be a noop)
        env.with_commit(|txn| {
            let reader = env.reader()?;
            let mut store = ChainCasBuf::primary(&reader, &env, false)?;
            store.put(header_pub, Some(entry_pub.clone()))?;
            store.put(header_priv, Some(entry_priv.clone()))?;
            store.flush_to_txn(txn)
        })?;

        // Can retrieve both entries when private entries are enabled
        {
            let reader = env.reader()?;
            let store = ChainCasBuf::primary(&reader, &env, true)?;
            assert_eq!(
                store.get_entry(entry_pub.as_hash().clone()),
                Ok(Some(entry_pub.as_content().clone()))
            );
            assert_eq!(store.get_entry(entry_priv.as_hash().clone()), Ok(None));
        }

        // Cannot retrieve private entry when disabled
        {
            let reader = env.reader()?;
            let store = ChainCasBuf::primary(&reader, &env, false)?;
            assert_eq!(
                store.get_entry(entry_pub.as_hash().clone()),
                Ok(Some(entry_pub.as_content().clone()))
            );
            assert_eq!(store.get_entry(entry_priv.as_hash().clone()), Ok(None));
        }

        Ok(())
    }
}
