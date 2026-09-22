//! Distribution dist-git management, remote search, and source package retrieval.

pub mod client;
pub mod provider;
pub mod spec;

pub use client::{DiscoveredProject, DistGitClient, GitRepoStatus};
pub use provider::{ApiType, DistroConfig};
pub use spec::{parse_spec_file, SpecMetadata};
