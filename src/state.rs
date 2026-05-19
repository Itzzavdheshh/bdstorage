use crate::types::{FileMetadata, Hash, hash_to_hex};
use anyhow::{Context, Result};
use crossbeam::channel::Receiver;
use redb::{Database, ReadableTable, TableDefinition};
use std::path::{Path, PathBuf};

const FILE_INDEX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("file_index");
const CAS_INDEX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("cas_index");
const VAULTED_INODES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("vaulted_inodes");
const BATCH_SIZE: usize = 1000;

#[derive(Clone, Debug)]
pub enum DbOp {
    UpsertFile(PathBuf, FileMetadata),
    SetCasRefcount(Hash, u64),
    MarkInodeVaulted(u64),
    RemoveFileFromIndex(PathBuf),
    UnmarkInodeVaulted(u64),
    RemoveCasRefcount(Hash),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VaultSummary {
    pub vault_location: String,
    pub objects_in_vault: usize,
    pub total_vault_size: u64,
    pub tracked_paths: usize,
    pub estimated_savings: u64,
    pub deduplication_ratio: f64,
}

#[derive(Clone)]
pub struct State {
    db: std::sync::Arc<Database>,
}

#[allow(dead_code)]
impl State {
    pub fn open_default() -> Result<Self> {
        Self::open_default_impl(false)
    }

    pub fn open_readonly_if_exists() -> Result<Self> {
        let db_path = default_db_path()?;
        if !db_path.exists() {
            return Self::create_dummy();
        }
        Self::open_default_impl(true)
    }

    fn create_dummy() -> Result<Self> {
        let temp_dir = std::env::temp_dir().join(format!("bdstorage-dry-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir)?;
        let db_path = temp_dir.join("dummy.redb");
        let db = Database::create(&db_path)?;
        let txn = db.begin_write()?;
        {
            let _ = txn.open_table(FILE_INDEX)?;
            let _ = txn.open_table(CAS_INDEX)?;
            let _ = txn.open_table(VAULTED_INODES)?;
        }
        txn.commit()?;
        Ok(Self {
            db: std::sync::Arc::new(db),
        })
    }

    fn open_default_impl(readonly: bool) -> Result<Self> {
        let db_path = default_db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create state directory {:?}", parent))?;
        }
        let db = if readonly {
            Database::open(&db_path).with_context(|| "open redb database")?
        } else {
            Database::create(&db_path).with_context(|| "open redb database")?
        };
        let txn = db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let _ = txn.open_table(FILE_INDEX)?;
            let _ = txn.open_table(CAS_INDEX)?;
            let _ = txn.open_table(VAULTED_INODES)?;
        }
        txn.commit()
            .with_context(|| "commit table initialization")?;
        Ok(Self {
            db: std::sync::Arc::new(db),
        })
    }

    pub fn upsert_file(&self, path: &Path, metadata: &FileMetadata) -> Result<()> {
        let key = path.to_string_lossy().as_bytes().to_vec();
        let value = bincode::serialize(metadata).with_context(|| "serialize file metadata")?;
        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let mut table = txn.open_table(FILE_INDEX)?;
            table.insert(key.as_slice(), value.as_slice())?;
        }
        txn.commit().with_context(|| "commit file index write")?;
        Ok(())
    }

    pub fn set_cas_refcount(&self, hash: &Hash, count: u64) -> Result<()> {
        let key = hash.to_vec();
        let value = count.to_le_bytes().to_vec();
        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let mut table = txn.open_table(CAS_INDEX)?;
            table.insert(key.as_slice(), value.as_slice())?;
        }
        txn.commit().with_context(|| "commit cas index write")?;
        Ok(())
    }

    pub fn is_inode_vaulted(&self, inode: u64) -> Result<bool> {
        let key = inode.to_le_bytes();
        let txn = self
            .db
            .begin_read()
            .with_context(|| "begin read transaction")?;
        let table = match txn.open_table(VAULTED_INODES) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        Ok(table.get(key.as_slice())?.is_some())
    }

    pub fn mark_inode_vaulted(&self, inode: u64) -> Result<()> {
        let key = inode.to_le_bytes();
        let value = 1u8;
        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let mut table = txn.open_table(VAULTED_INODES)?;
            table.insert(key.as_slice(), std::slice::from_ref(&value))?;
        }
        txn.commit().with_context(|| "commit vaulted inode write")?;
        Ok(())
    }

    pub fn get_file_metadata(&self, path: &Path) -> Result<Option<FileMetadata>> {
        let key = path.to_string_lossy().as_bytes().to_vec();
        let txn = self
            .db
            .begin_read()
            .with_context(|| "begin read transaction")?;
        let table = match txn.open_table(FILE_INDEX) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        if let Some(access) = table.get(key.as_slice())? {
            let metadata: FileMetadata = bincode::deserialize(access.value())
                .with_context(|| "deserialize file metadata")?;
            return Ok(Some(metadata));
        }
        Ok(None)
    }

    pub fn remove_file_from_index(&self, path: &Path) -> Result<()> {
        let key = path.to_string_lossy().as_bytes().to_vec();
        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let mut table = txn.open_table(FILE_INDEX)?;
            table.remove(key.as_slice())?;
        }
        txn.commit().with_context(|| "commit file index removal")?;
        Ok(())
    }

    pub fn unmark_inode_vaulted(&self, inode: u64) -> Result<()> {
        let key = inode.to_le_bytes();
        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let mut table = txn.open_table(VAULTED_INODES)?;
            table.remove(key.as_slice())?;
        }
        txn.commit()
            .with_context(|| "commit unmark vaulted inode")?;
        Ok(())
    }

    pub fn get_cas_refcount(&self, hash: &Hash) -> Result<u64> {
        let key = hash.to_vec();
        let txn = self
            .db
            .begin_read()
            .with_context(|| "begin read transaction")?;
        let table = match txn.open_table(CAS_INDEX) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
            Err(err) => return Err(err.into()),
        };
        if let Some(access) = table.get(key.as_slice())? {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(access.value());
            return Ok(u64::from_le_bytes(bytes));
        }
        Ok(0)
    }

    pub fn remove_cas_refcount(&self, hash: &Hash) -> Result<()> {
        let key = hash.to_vec();
        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin write transaction")?;
        {
            let mut table = txn.open_table(CAS_INDEX)?;
            table.remove(key.as_slice())?;
        }
        txn.commit().with_context(|| "commit cas index removal")?;
        Ok(())
    }

    pub fn batch_write(&self, ops: Vec<DbOp>) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }

        let txn = self
            .db
            .begin_write()
            .with_context(|| "begin batch write transaction")?;
        {
            for op in ops {
                match op {
                    DbOp::UpsertFile(path, metadata) => {
                        let key = path.to_string_lossy().as_bytes().to_vec();
                        let value = bincode::serialize(&metadata)
                            .with_context(|| "serialize file metadata")?;
                        let mut table = txn.open_table(FILE_INDEX)?;
                        table.insert(key.as_slice(), value.as_slice())?;
                    }
                    DbOp::SetCasRefcount(hash, count) => {
                        let key = hash.to_vec();
                        let value = count.to_le_bytes().to_vec();
                        let mut table = txn.open_table(CAS_INDEX)?;
                        table.insert(key.as_slice(), value.as_slice())?;
                    }
                    DbOp::MarkInodeVaulted(inode) => {
                        let key = inode.to_le_bytes();
                        let value = 1u8;
                        let mut table = txn.open_table(VAULTED_INODES)?;
                        table.insert(key.as_slice(), std::slice::from_ref(&value))?;
                    }
                    DbOp::RemoveFileFromIndex(path) => {
                        let key = path.to_string_lossy().as_bytes().to_vec();
                        let mut table = txn.open_table(FILE_INDEX)?;
                        table.remove(key.as_slice())?;
                    }
                    DbOp::UnmarkInodeVaulted(inode) => {
                        let key = inode.to_le_bytes();
                        let mut table = txn.open_table(VAULTED_INODES)?;
                        table.remove(key.as_slice())?;
                    }
                    DbOp::RemoveCasRefcount(hash) => {
                        let key = hash.to_vec();
                        let mut table = txn.open_table(CAS_INDEX)?;
                        table.remove(key.as_slice())?;
                    }
                }
            }
        }
        txn.commit()
            .with_context(|| "commit batch write transaction")?;
        Ok(())
    }

    pub fn batch_write_from_channel(&self, rx: Receiver<DbOp>) {
        let mut buffer = Vec::with_capacity(BATCH_SIZE);

        loop {
            buffer.clear();

            for _ in 0..BATCH_SIZE {
                match rx.try_recv() {
                    Ok(op) => buffer.push(op),
                    Err(_) => break,
                }
            }

            if buffer.is_empty() {
                match rx.recv() {
                    Ok(op) => {
                        buffer.push(op);

                        while let Ok(op) = rx.try_recv() {
                            buffer.push(op);
                            if buffer.len() >= BATCH_SIZE {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }

            if !buffer.is_empty() {
                let _ = self.batch_write(std::mem::take(&mut buffer));
            }
        }
    }

    pub fn compute_summary(&self, vault_location: &Path) -> Result<VaultSummary> {
        if !vault_location.exists() {
            anyhow::bail!("No vault exists yet. Please run dedupe first.");
        }

        let txn = self
            .db
            .begin_read()
            .with_context(|| "begin read transaction")?;

        let file_table = match txn.open_table(FILE_INDEX) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                anyhow::bail!("No vault exists yet. Please run dedupe first.");
            }
            Err(e) => return Err(e.into()),
        };

        let mut tracked_paths = 0;
        let mut unique_hashes = std::collections::HashMap::new();

        for result in file_table.iter()? {
            let (_, value) = result?;
            let metadata: FileMetadata = bincode::deserialize(value.value())?;
            tracked_paths += 1;
            unique_hashes.insert(metadata.hash, metadata.size);
        }

        let mut objects_in_vault = 0;
        let mut total_vault_size = 0;
        let mut estimated_savings = 0;

        if let Ok(cas_table) = txn.open_table(CAS_INDEX) {
            for result in cas_table.iter()? {
                let (key, value) = result?;
                let mut hash = [0u8; 32];
                hash.copy_from_slice(key.value());

                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(value.value());
                let refcount = u64::from_le_bytes(bytes);

                if refcount > 0 && let Some(&size) = unique_hashes.get(&hash) {
                    let object_path = vault_object_path(vault_location, &hash);
                    let Ok(object_metadata) = std::fs::metadata(object_path) else {
                        continue;
                    };
                    let object_size = object_metadata.len();
                    objects_in_vault += 1;
                    total_vault_size += object_size;
                    estimated_savings += size.saturating_mul(refcount.saturating_sub(1));
                }
            }
        }

        let deduplication_ratio = if total_vault_size > 0 {
            (total_vault_size as f64 + estimated_savings as f64) / (total_vault_size as f64)
        } else {
            1.0
        };

        Ok(VaultSummary {
            vault_location: vault_location.to_string_lossy().into_owned(),
            objects_in_vault,
            total_vault_size,
            tracked_paths,
            estimated_savings,
            deduplication_ratio,
        })
    }
}

fn vault_object_path(vault_location: &Path, hash: &Hash) -> PathBuf {
    let hex = hash_to_hex(hash);
    vault_location.join(&hex[0..2]).join(&hex[2..4]).join(hex)
}

pub fn default_db_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .with_context(|| "Neither HOME nor USERPROFILE is set")?;
    Ok(PathBuf::from(home).join(".imprint").join("state.redb"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_compute_summary() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.redb");
        let db = Database::create(&db_path)?;

        let txn = db.begin_write()?;
        {
            let _ = txn.open_table(FILE_INDEX)?;
            let _ = txn.open_table(CAS_INDEX)?;
            let _ = txn.open_table(VAULTED_INODES)?;
        }
        txn.commit()?;

        let state = State {
            db: std::sync::Arc::new(db),
        };

        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];
        let hash3 = [3u8; 32];

        // 4 files tracked:
        // file1: hash1, size 100
        // file2: hash1, size 100
        // file3: hash2, size 50
        // file4: hash3, size 200 (not deduplicated)
        state.batch_write(vec![
            DbOp::UpsertFile(
                PathBuf::from("f1"),
                FileMetadata {
                    size: 100,
                    modified: 0,
                    hash: hash1,
                },
            ),
            DbOp::UpsertFile(
                PathBuf::from("f2"),
                FileMetadata {
                    size: 100,
                    modified: 0,
                    hash: hash1,
                },
            ),
            DbOp::UpsertFile(
                PathBuf::from("f3"),
                FileMetadata {
                    size: 50,
                    modified: 0,
                    hash: hash2,
                },
            ),
            DbOp::UpsertFile(
                PathBuf::from("f4"),
                FileMetadata {
                    size: 200,
                    modified: 0,
                    hash: hash3,
                },
            ),
            DbOp::SetCasRefcount(hash1, 2),
            DbOp::SetCasRefcount(hash2, 1),
        ])?;

        let vault_dir = dir.path().join("vault");
        let hash1_hex = hash_to_hex(&hash1);
        let hash2_hex = hash_to_hex(&hash2);
        std::fs::create_dir_all(vault_dir.join(&hash1_hex[0..2]).join(&hash1_hex[2..4]))?;
        std::fs::create_dir_all(vault_dir.join(&hash2_hex[0..2]).join(&hash2_hex[2..4]))?;
        std::fs::write(vault_object_path(&vault_dir, &hash1), vec![1u8; 100])?;
        std::fs::write(vault_object_path(&vault_dir, &hash2), vec![2u8; 50])?;

        let summary = state.compute_summary(&vault_dir)?;

        assert_eq!(summary.tracked_paths, 4);
        assert_eq!(summary.objects_in_vault, 2);
        assert_eq!(summary.total_vault_size, 150); // hash1 (100) + hash2 (50)
        assert_eq!(summary.estimated_savings, 100); // hash1 saves 100, hash2 saves 0

        let expected_ratio = (150.0 + 100.0) / 150.0;
        assert!((summary.deduplication_ratio - expected_ratio).abs() < f64::EPSILON);

        Ok(())
    }

    #[test]
    fn test_compute_summary_ignores_missing_vault_objects() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.redb");
        let db = Database::create(&db_path)?;

        let txn = db.begin_write()?;
        {
            let _ = txn.open_table(FILE_INDEX)?;
            let _ = txn.open_table(CAS_INDEX)?;
            let _ = txn.open_table(VAULTED_INODES)?;
        }
        txn.commit()?;

        let state = State {
            db: std::sync::Arc::new(db),
        };
        let hash = [7u8; 32];
        let vault_dir = dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir)?;

        state.batch_write(vec![
            DbOp::UpsertFile(
                PathBuf::from("f1"),
                FileMetadata {
                    size: 100,
                    modified: 0,
                    hash,
                },
            ),
            DbOp::UpsertFile(
                PathBuf::from("f2"),
                FileMetadata {
                    size: 100,
                    modified: 0,
                    hash,
                },
            ),
            DbOp::SetCasRefcount(hash, 2),
        ])?;

        let summary = state.compute_summary(&vault_dir)?;

        assert_eq!(summary.tracked_paths, 2);
        assert_eq!(summary.objects_in_vault, 0);
        assert_eq!(summary.total_vault_size, 0);
        assert_eq!(summary.estimated_savings, 0);
        assert_eq!(summary.deduplication_ratio, 1.0);

        Ok(())
    }
}
