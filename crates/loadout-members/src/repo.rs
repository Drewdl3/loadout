//! `repo_access` provider: member iff the group's first source can be read
//! (`git ls-remote`), using the user's own Git credentials.

use loadout_git::{Git, GitError};

use crate::{ProviderError, RepoAccess};

pub struct GitRepoAccess<'a>(pub &'a Git);

impl RepoAccess for GitRepoAccess<'_> {
    fn can_access(&self, url: &str) -> Result<bool, ProviderError> {
        match self.0.can_read(url) {
            Ok(ok) => Ok(ok),
            Err(e @ GitError::Spawn { .. }) => {
                Err(ProviderError::new("repo_access", e.to_string()))
            }
            Err(_) => Ok(false),
        }
    }
}
