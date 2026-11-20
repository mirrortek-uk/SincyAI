//! Object storage abstraction. Production: S3-compatible with pre-signed URLs.
//! Dev/test: local filesystem under a root dir (same semantics: put/get/delete).

use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("object not found: {0}")]
    NotFound(String),
}

pub trait ObjectStore: Send + Sync {
    fn put(
        &self,
        key: &str,
        bytes: &[u8],
    ) -> impl std::future::Future<Output = Result<(), StoreError>> + Send;
    fn get(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, StoreError>> + Send;
    fn delete(&self, key: &str)
    -> impl std::future::Future<Output = Result<(), StoreError>> + Send;
}

/// Local filesystem object store (dev). Keys are relative paths under root.
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    fn path_for(&self, key: &str) -> Result<PathBuf, StoreError> {
        // prevent path traversal
        let p = self.root.join(key);
        if !p.starts_with(&self.root) {
            return Err(StoreError::NotFound(key.to_string()));
        }
        Ok(p)
    }
}

impl ObjectStore for LocalObjectStore {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let p = self.path_for(key)?;
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(p, bytes).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StoreError> {
        let p = self.path_for(key)?;
        fs::read(p).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound(key.to_string())
            } else {
                StoreError::Io(e)
            }
        })
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        let p = self.path_for(key)?;
        match fs::remove_file(p).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Io(e)),
        }
    }
}
