use hdk3::prelude::*;

#[hdk_entry(id = "countree")]
/// a tree of counters
#[derive(Default, Clone, Copy, PartialEq)]
pub struct CounTree(u32);

impl std::ops::Add for CounTree {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl CounTree {
    #[allow(clippy::new_ret_no_self)]
    /// ensures that a default countree exists and returns the header
    pub fn new() -> ExternResult<HeaderHash> {
        Self::ensure(Self::default())
    }

    /// commits if not exists else returns found header
    /// produces redundant headers in a partition
    pub fn ensure(countree: CounTree) -> ExternResult<HeaderHash> {
        match get!(entry_hash!(countree)?)? {
            Some(element) => Ok(element.header_address().to_owned()),
            None => Ok(commit_entry!(countree)?)
        }
    }

    pub fn header_details(header_hash: HeaderHash) -> ExternResult<GetDetailsOutput> {
        Ok(GetDetailsOutput::new(get_details!(header_hash)?))
    }

    /// return the GetDetailsOutput for the entry hash from the header
    pub fn entry_details(entry_hash: EntryHash) -> ExternResult<GetDetailsOutput> {
        Ok(GetDetailsOutput::new(get_details!(entry_hash)?))
    }

    /// increments the given header hash by 1 or creates it if not found
    /// this is silly as being offline resets the counter >.<
    pub fn incsert(header_hash: HeaderHash) -> ExternResult<HeaderHash> {
        let current: CounTree = match get!(header_hash.clone())? {
            Some(element) => {
                match element.entry().to_app_option()? {
                    Some(v) => v,
                    None => return Self::new(),
                }
            },
            None => return Self::new(),
        };

        Ok(update_entry!(header_hash, current + CounTree(1))?)
    }

    pub fn dec(header_hash: HeaderHash) -> ExternResult<HeaderHash> {
        Ok(delete_entry!(header_hash)?)
    }
}
