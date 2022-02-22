use crate::prelude::*;

/// Hash anything that implements [`TryInto<Entry>`].
///
/// Hashes are typed in Holochain, e.g. [`HeaderHash`] and [`EntryHash`] are different and yield different
/// bytes for a given value. This ensures correctness and allows type based dispatch in various
/// areas of the codebase.
///
/// Usually you want to hash a value that you want to reference on the DHT with [`get`] etc. because
/// it represents some domain-specific data sourced externally or generated within the wasm.
/// [`HeaderHash`] hashes are _always_ generated by the process of committing something to a local
/// chain. Every host function that commits an entry returns the new [`HeaderHash`]. The [`HeaderHash`] can
/// also be used with [`get`] etc. to retreive a _specific_ element from the DHT rather than the
/// oldest live element.
/// However there is no way to _generate_ a header hash directly from a header from inside wasm.
/// [`Element`] values (entry+header pairs returned by [`get`] etc.) contain prehashed header structs
/// called [`HeaderHashed`], which is composed of a [`HeaderHash`] alongside the "raw" [`Header`] value. Generally the pre-hashing is
/// more efficient than hashing headers ad-hoc as hashing always needs to be done at the database
/// layer, so we want to re-use that as much as possible.
/// The header hash can be extracted from the Element as `element.header_hashed().as_hash()`.
///
/// @todo is there any use-case that can't be satisfied by the `header_hashed` approach?
///
/// Anything that is annotated with #[hdk_entry( .. )] or entry_def!( .. ) implements this so is
/// compatible automatically.
///
/// [`hash_entry`] is "dumb" in that it doesn't check that the entry is defined, committed, on the DHT or
/// any other validation, it simply generates the hash for the serialized representation of
/// something in the same way that the DHT would.
///
/// It is strongly recommended that you use the [`hash_entry`] function to calculate hashes to avoid
/// inconsistencies between hashes in the wasm guest and the host.
/// For example, a lot of the crypto crates in rust compile to wasm so in theory could generate the
/// hash in the guest, but there is the potential that the serialization logic could be slightly
/// different, etc.
///
/// ```ignore
/// #[hdk_entry(id="foo")]
/// struct Foo;
///
/// let foo_hash = hash_entry(Foo)?;
/// ```
pub fn hash_entry<I, E>(input: I) -> ExternResult<EntryHash>
where
    Entry: TryFrom<I, Error = E>,
    WasmError: From<E>,
{
    match HDK.with(|h| h.borrow().hash(HashInput::Entry(Entry::try_from(input)?)))? {
        HashOutput::Entry(entry_hash) => Ok(entry_hash),
        _ => unreachable!(),
    }
}

/// Hash a `Header` into a `HeaderHash`.
///
/// [`hash_entry`] has more of a discussion around different hash types and how
/// they are used within the HDK.
///
/// It is strongly recommended to use [`hash_header`] to calculate the hash rather than hand rolling an in-wasm solution.
/// Any inconsistencies in serialization or hash handling will result in dangling references to things due to a "corrupt" hash.
///
/// Note that usually relevant HDK functions return a [`HeaderHashed`] or [`SignedHeaderHashed`] which already has associated methods to access the `HeaderHash` of the inner `Header`.
/// In normal usage it is unlikely to be required to separately hash a [`Header`] like this.
pub fn hash_header(input: Header) -> ExternResult<HeaderHash> {
    match HDK.with(|h| h.borrow().hash(HashInput::Header(input)))? {
        HashOutput::Header(header_hash) => Ok(header_hash),
        _ => unreachable!(),
    }
}

/// Hash arbitrary bytes using BLAKE2b.
/// This is the same algorithm used by holochain for typed hashes.
/// Notably the output hash length is configurable.
pub fn hash_blake2b(input: Vec<u8>, output_len: u8) -> ExternResult<Vec<u8>> {
    match HDK.with(|h| h.borrow().hash(HashInput::Blake2B(input, output_len)))? {
        HashOutput::Blake2B(vec) => Ok(vec),
        _ => unreachable!(),
    }
}
