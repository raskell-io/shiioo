use crate::types::BlobHash;
use anyhow::{Context, Result};
use bytes::Bytes;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

/// Content-addressed blob storage abstraction
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync {
    /// Store a blob and return its content hash
    async fn put(&self, data: Bytes) -> Result<BlobHash>;

    /// Retrieve a blob by its content hash
    async fn get(&self, hash: &BlobHash) -> Result<Option<Bytes>>;

    /// Check if a blob exists
    async fn exists(&self, hash: &BlobHash) -> Result<bool>;

    /// Delete a blob (for garbage collection)
    async fn delete(&self, hash: &BlobHash) -> Result<()>;
}

/// Filesystem-based blob store (for local development and single-node deployments)
#[derive(Clone)]
pub struct FilesystemBlobStore {
    base_path: PathBuf,
}

impl FilesystemBlobStore {
    pub fn new(base_path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&base_path)
            .context("Failed to create blob store directory")?;
        Ok(Self { base_path })
    }

    fn blob_path(&self, hash: &BlobHash) -> PathBuf {
        // Store blobs in subdirectories based on first 2 chars of hash (like Git)
        // e.g., blobs/ab/abcdef123...
        let hash_str = &hash.0;
        let prefix = &hash_str[..2];
        self.base_path.join(prefix).join(hash_str)
    }
}

#[async_trait::async_trait]
impl BlobStore for FilesystemBlobStore {
    async fn put(&self, data: Bytes) -> Result<BlobHash> {
        let hash = BlobHash::from_bytes(&data);
        let path = self.blob_path(&hash);

        // Create parent directory
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create blob directory")?;
        }

        // Write blob to disk (only if it doesn't exist - content-addressed)
        if !path.exists() {
            let mut file = tokio::fs::File::create(&path)
                .await
                .context("Failed to create blob file")?;
            file.write_all(&data)
                .await
                .context("Failed to write blob")?;
            file.sync_all().await.context("Failed to sync blob")?;
        }

        Ok(hash)
    }

    async fn get(&self, hash: &BlobHash) -> Result<Option<Bytes>> {
        let path = self.blob_path(hash);
        if !path.exists() {
            return Ok(None);
        }

        let data = tokio::fs::read(&path)
            .await
            .context("Failed to read blob")?;
        Ok(Some(Bytes::from(data)))
    }

    async fn exists(&self, hash: &BlobHash) -> Result<bool> {
        Ok(self.blob_path(hash).exists())
    }

    async fn delete(&self, hash: &BlobHash) -> Result<()> {
        let path = self.blob_path(hash);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .context("Failed to delete blob")?;
        }
        Ok(())
    }
}

/// Object store-based blob store (S3, MinIO, etc.)
pub struct ObjectStoreBlobStore {
    store: Box<dyn object_store::ObjectStore>,
    prefix: String,
}

impl ObjectStoreBlobStore {
    pub fn new(store: Box<dyn object_store::ObjectStore>, prefix: impl Into<String>) -> Self {
        Self {
            store,
            prefix: prefix.into(),
        }
    }

    fn blob_key(&self, hash: &BlobHash) -> object_store::path::Path {
        let hash_str = &hash.0;
        let prefix = &hash_str[..2];
        let key = format!("{}/blobs/{}/{}", self.prefix, prefix, hash_str);
        object_store::path::Path::from(key)
    }
}

#[async_trait::async_trait]
impl BlobStore for ObjectStoreBlobStore {
    async fn put(&self, data: Bytes) -> Result<BlobHash> {
        let hash = BlobHash::from_bytes(&data);
        let key = self.blob_key(&hash);

        // Check if blob already exists (content-addressed)
        if self.store.head(&key).await.is_ok() {
            return Ok(hash);
        }

        self.store
            .put(&key, data.into())
            .await
            .context("Failed to put blob to object store")?;

        Ok(hash)
    }

    async fn get(&self, hash: &BlobHash) -> Result<Option<Bytes>> {
        let key = self.blob_key(hash);

        match self.store.get(&key).await {
            Ok(result) => {
                let bytes = result.bytes().await.context("Failed to read blob bytes")?;
                Ok(Some(bytes))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(e).context("Failed to get blob from object store"),
        }
    }

    async fn exists(&self, hash: &BlobHash) -> Result<bool> {
        let key = self.blob_key(hash);
        match self.store.head(&key).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(e).context("Failed to check blob existence"),
        }
    }

    async fn delete(&self, hash: &BlobHash) -> Result<()> {
        let key = self.blob_key(hash);
        self.store
            .delete(&key)
            .await
            .context("Failed to delete blob from object store")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_filesystem_blob_store() {
        let temp_dir = TempDir::new().unwrap();
        let store = FilesystemBlobStore::new(temp_dir.path().to_path_buf()).unwrap();

        let data = Bytes::from("Hello, world!");
        let hash = store.put(data.clone()).await.unwrap();

        assert!(store.exists(&hash).await.unwrap());

        let retrieved = store.get(&hash).await.unwrap().unwrap();
        assert_eq!(retrieved, data);

        store.delete(&hash).await.unwrap();
        assert!(!store.exists(&hash).await.unwrap());
    }
}
