pub fn crate_name() -> &'static str {
    "config"
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    Economy,
    #[default]
    Balanced,
    Turbo,
}

impl Profile {
    pub fn defaults(self) -> ProfileDefaults {
        match self {
            Profile::Economy => ProfileDefaults {
                profile: self,
                max_worker_threads: 1,
                max_parallel_documents: 1,
                index_commit_batch_size: 64,
                max_document_bytes: 2 * 1024 * 1024,
            },
            Profile::Balanced => ProfileDefaults {
                profile: self,
                max_worker_threads: 4,
                max_parallel_documents: 4,
                index_commit_batch_size: 256,
                max_document_bytes: 8 * 1024 * 1024,
            },
            Profile::Turbo => ProfileDefaults {
                profile: self,
                max_worker_threads: 8,
                max_parallel_documents: 8,
                index_commit_batch_size: 1024,
                max_document_bytes: 32 * 1024 * 1024,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileDefaults {
    pub profile: Profile,
    pub max_worker_threads: usize,
    pub max_parallel_documents: usize,
    pub index_commit_batch_size: usize,
    pub max_document_bytes: usize,
}

impl Default for ProfileDefaults {
    fn default() -> Self {
        Profile::default().defaults()
    }
}
