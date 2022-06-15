use hdk::prelude::*;

mod countree;

#[hdk_dependent_entry_types]
enum EntryZomes {
    IntegrityCrud(crate::integrity::EntryTypes),
}

#[hdk_extern]
fn new(_: ()) -> ExternResult<HeaderHash> {
    countree::CounTree::new()
}

#[hdk_extern]
fn header_details(header_hashes: Vec<HeaderHash>) -> ExternResult<Vec<Option<Details>>> {
    countree::CounTree::header_details(header_hashes)
}

#[hdk_extern]
fn entry_details(entry_hashes: Vec<EntryHash>) -> ExternResult<Vec<Option<Details>>> {
    countree::CounTree::entry_details(entry_hashes)
}

#[hdk_extern]
fn entry_hash(countree: countree::CounTree) -> ExternResult<EntryHash> {
    hash_entry(&countree)
}

#[hdk_extern]
fn inc(header_hash: HeaderHash) -> ExternResult<HeaderHash> {
    countree::CounTree::incsert(header_hash)
}

#[hdk_extern]
fn dec(header_hash: HeaderHash) -> ExternResult<HeaderHash> {
    countree::CounTree::dec(header_hash)
}
