//! 加密 SQLite 持久化存储引擎。
//!
//! 提供基于 AES-256-GCM 的文档级静态数据加密、乐观锁版本控制（Revision）、
//! 租户配额限制以及跨表事务原子导入能力。

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use anyhow::{Context, Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

/// 线程安全的本地加密 SQLite 存储连接。
#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    cipher: Arc<Aes256Gcm>,
}

impl Store {
    /// 打开或初始化指定路径的 SQLite 数据库，并加载同名的 AES-256 主密钥。
    pub async fn open(path: &Path, busy_timeout: Duration) -> Result<Self> {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).context("Creating SQLite directory")?;
            }
            let key_path = path.with_extension("key");
            // Losing this key makes encrypted subscriptions unrecoverable; never replace it.
            let key = load_key(&path, &key_path)?;
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open_private(&path)?;
            drop(file);
            let connection = Connection::open(&path).context("Opening SQLite database")?;
            connection.busy_timeout(busy_timeout)?;
            connection.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;
                CREATE TABLE IF NOT EXISTS documents (
                    kind TEXT NOT NULL, id TEXT NOT NULL, owner TEXT NOT NULL, token TEXT UNIQUE,
                    data TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 1,
                    PRIMARY KEY(kind,id));
                CREATE INDEX IF NOT EXISTS documents_owner ON documents(kind,owner);
                CREATE TABLE IF NOT EXISTS sessions (
                    token TEXT PRIMARY KEY, owner TEXT NOT NULL, username TEXT NOT NULL,
                    guest INTEGER NOT NULL, expires INTEGER NOT NULL);
                CREATE INDEX IF NOT EXISTS sessions_expiry ON sessions(expires);
                CREATE TABLE IF NOT EXISTS admin (
                    singleton INTEGER PRIMARY KEY CHECK(singleton=1), id TEXT UNIQUE NOT NULL,
                    username TEXT UNIQUE NOT NULL, password_hash TEXT NOT NULL);
                PRAGMA user_version=1;",
            )?;
            let cipher =
                Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("Invalid encryption key"))?;
            Ok(Self {
                connection: Arc::new(Mutex::new(connection)),
                cipher: Arc::new(cipher),
            })
        })
        .await?
    }

    pub async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow!("SQLite lock poisoned"))?;
            operation(&mut connection)
        })
        .await?
    }

    pub(crate) fn encrypt(&self, value: &Value) -> Result<String> {
        let nonce: [u8; 12] = rand::random();
        let encrypted = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                serde_json::to_vec(value)?.as_slice(),
            )
            .map_err(|_| anyhow!("Unable to encrypt document"))?;
        let mut bytes = Vec::with_capacity(12 + encrypted.len());
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&encrypted);
        Ok(STANDARD.encode(bytes))
    }

    fn decrypt(&self, data: &str) -> Result<Value> {
        let bytes = STANDARD
            .decode(data)
            .context("Invalid stored document encoding")?;
        ensure!(bytes.len() >= 28, "Invalid encrypted document");
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
            .map_err(|_| anyhow!("Unable to decrypt document; check the adjacent .key file"))?;
        Ok(serde_json::from_slice(&plaintext)?)
    }

    pub async fn list(&self, kind: &str, owner: Option<&str>) -> Result<Vec<(Value, i64)>> {
        let kind = kind.to_owned();
        let owner = owner.map(str::to_owned);
        let rows = self.run(move |db| {
            let mut query = db.prepare("SELECT data,revision FROM documents WHERE kind=?1 AND (?2 IS NULL OR owner=?2) ORDER BY rowid DESC")?;
            let rows = query.query_map(params![kind, owner], |row| Ok((row.get::<_,String>(0)?, row.get::<_,i64>(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        rows.into_iter()
            .map(|(data, revision)| Ok((self.decrypt(&data)?, revision)))
            .collect()
    }

    pub async fn get(&self, kind: &str, owner: &str, id: &str) -> Result<Option<(Value, i64)>> {
        let (kind, owner, id) = (kind.to_owned(), owner.to_owned(), id.to_owned());
        let row = self
            .run(move |db| {
                Ok(db
                    .query_row(
                        "SELECT data,revision FROM documents WHERE kind=?1 AND owner=?2 AND id=?3",
                        params![kind, owner, id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?)
            })
            .await?;
        row.map(|(data, revision)| Ok((self.decrypt(&data)?, revision)))
            .transpose()
    }

    pub async fn by_token(&self, token: &str) -> Result<Option<(String, Value)>> {
        let token = token.to_owned();
        let row = self
            .run(move |db| {
                Ok(db
                    .query_row(
                        "SELECT owner,data FROM documents WHERE kind='subscription' AND token=?1",
                        [token],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?)
            })
            .await?;
        row.map(|(owner, data)| Ok((owner, self.decrypt(&data)?)))
            .transpose()
    }

    pub async fn count(&self, kind: &str, owner: &str) -> Result<usize> {
        let (kind, owner) = (kind.to_owned(), owner.to_owned());
        self.run(move |db| {
            let count: i64 = db.query_row(
                "SELECT COUNT(*) FROM documents WHERE kind=?1 AND owner=?2",
                params![kind, owner],
                |row| row.get(0),
            )?;
            usize::try_from(count).context("Document count exceeds addressable size")
        })
        .await
    }

    pub async fn insert(
        &self,
        kind: &str,
        owner: &str,
        document: &Value,
        maximum: usize,
    ) -> Result<()> {
        let data = self.encrypt(document)?;
        let id = document["id"]
            .as_str()
            .context("Document ID required")?
            .to_owned();
        let token = document["token"].as_str().map(str::to_owned);
        let (kind, owner) = (kind.to_owned(), owner.to_owned());
        self.run(move |db| {
            let transaction =
                db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM documents WHERE kind=?1 AND owner=?2",
                params![kind, owner],
                |row| row.get(0),
            )?;
            ensure!(
                count < i64::try_from(maximum).unwrap_or(i64::MAX),
                crate::service::fault(
                    403,
                    "QUOTA_EXCEEDED",
                    format!("Maximum {maximum} saved {kind}s")
                )
            );
            transaction.execute(
                "INSERT INTO documents(kind,id,owner,token,data) VALUES(?1,?2,?3,?4,?5)",
                params![kind, id, owner, token, data],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn update(
        &self,
        kind: &str,
        owner: &str,
        document: &Value,
        revision: i64,
    ) -> Result<()> {
        let data = self.encrypt(document)?;
        let id = document["id"]
            .as_str()
            .context("Document ID required")?
            .to_owned();
        let token = document["token"].as_str().map(str::to_owned);
        let (kind, owner) = (kind.to_owned(), owner.to_owned());
        self.run(move |db| {
            let changed = db.execute("UPDATE documents SET data=?1,token=?2,revision=revision+1 WHERE kind=?3 AND owner=?4 AND id=?5 AND revision=?6",params![data,token,kind,owner,id,revision])?;
            ensure!(changed == 1, crate::service::fault(409,"CONFLICT","Document changed while operation was in progress"));
            Ok(())
        }).await
    }

    pub async fn delete(&self, kind: &str, owner: &str, id: &str) -> Result<bool> {
        let (kind, owner, id) = (kind.to_owned(), owner.to_owned(), id.to_owned());
        self.run(move |db| {
            Ok(db.execute(
                "DELETE FROM documents WHERE kind=?1 AND owner=?2 AND id=?3",
                params![kind, owner, id],
            )? == 1)
        })
        .await
    }
}

trait PrivateOpen {
    fn open_private(&mut self, path: &Path) -> std::io::Result<std::fs::File>;
}
impl PrivateOpen for OpenOptions {
    fn open_private(&mut self, path: &Path) -> std::io::Result<std::fs::File> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            self.mode(0o600);
        }
        self.open(path)
    }
}

impl Store {
    pub async fn owner_of(&self, id: &str) -> Result<Option<String>> {
        let id = id.to_owned();
        self.run(move |db| {
            Ok(db
                .query_row(
                    "SELECT owner FROM documents WHERE kind='subscription' AND id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?)
        })
        .await
    }

    pub async fn import_atomic(
        &self,
        owner: &str,
        subscriptions: &[Value],
        templates: &[Value],
        maximum: usize,
    ) -> Result<()> {
        let mut records = Vec::with_capacity(subscriptions.len() + templates.len());
        for (kind, rows) in [("subscription", subscriptions), ("template", templates)] {
            for row in rows {
                records.push((
                    kind,
                    row["id"].as_str().context("Missing ID")?.to_owned(),
                    row["token"].as_str().map(str::to_owned),
                    self.encrypt(row)?,
                ));
            }
        }
        let owner = owner.to_owned();
        self.run(move |db| {
            let transaction = db.transaction()?;
            for (kind, extra) in [
                (
                    "subscription",
                    subscriptions_count(&records, "subscription"),
                ),
                ("template", subscriptions_count(&records, "template")),
            ] {
                let existing: i64 = transaction.query_row(
                    "SELECT count(*) FROM documents WHERE kind=?1 AND owner=?2",
                    params![kind, owner],
                    |r| r.get(0),
                )?;
                ensure!(
                    usize::try_from(existing)?
                        .checked_add(extra)
                        .is_some_and(|count| count <= maximum),
                    "Import exceeds saved document quota"
                );
            }
            for (kind, id, token, data) in records {
                transaction.execute(
                    "INSERT INTO documents(kind,id,owner,token,data) VALUES(?1,?2,?3,?4,?5)",
                    params![kind, id, owner, token, data],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }
}

fn subscriptions_count(records: &[(&str, String, Option<String>, String)], kind: &str) -> usize {
    records.iter().filter(|row| row.0 == kind).count()
}

fn load_key(database: &Path, path: &Path) -> Result<[u8; 32]> {
    if !path.exists() {
        ensure!(
            !database.metadata().is_ok_and(|m| m.len() > 0),
            "Encryption key missing; restore {} from backup before opening this database",
            path.display()
        );
        let temporary = path.with_extension(format!("key.{}", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open_private(&temporary)?;
        let key: [u8; 32] = rand::random();
        let result = (|| -> Result<()> {
            file.write_all(&key)?;
            file.sync_all()?;
            match std::fs::hard_link(&temporary, path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(error).context("Installing encryption key"),
            }
        })();
        drop(file);
        std::fs::remove_file(&temporary)?;
        result?;
    }
    let mut key = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut key)?;
    key.try_into()
        .map_err(|_| anyhow!("Invalid encryption key file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn persistence_encryption_owner_and_revision_are_enforced() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.sqlite3");
        let store = Store::open(&path, Duration::from_secs(1)).await?;
        let document =
            json!({"id":"a","token":"secret-token","nodes":[{"password":"secret-password"}]});
        store.insert("subscription", "alice", &document, 1).await?;
        assert!(store.get("subscription", "bob", "a").await?.is_none());
        store.update("subscription", "alice", &document, 1).await?;
        assert!(
            store
                .update("subscription", "alice", &document, 1)
                .await
                .is_err()
        );
        drop(store);
        let store = Store::open(&path, Duration::from_secs(1)).await?;
        assert_eq!(
            store
                .get("subscription", "alice", "a")
                .await?
                .context("missing row")?
                .0,
            document
        );
        let data = store
            .run(
                |db| Ok(db.query_row("SELECT data FROM documents", [], |r| r.get::<_, String>(0))?),
            )
            .await?;
        assert!(!data.contains("secret-password"));
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_connections_cannot_exceed_the_document_quota() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("quota.sqlite3");
        let first = Store::open(&path, Duration::from_secs(1)).await?;
        let second = Store::open(&path, Duration::from_secs(1)).await?;
        let first_document = json!({"id":"first","token":"first-token"});
        let second_document = json!({"id":"second","token":"second-token"});
        let (left, right) = tokio::join!(
            first.insert("subscription", "guest:quota", &first_document, 1),
            second.insert("subscription", "guest:quota", &second_document, 1)
        );
        assert_ne!(left.is_ok(), right.is_ok());
        let error = left
            .err()
            .or_else(|| right.err())
            .context("Expected quota rejection")?;
        assert_eq!(
            error
                .downcast_ref::<crate::service::Fault>()
                .map(|fault| fault.status),
            Some(403)
        );
        assert_eq!(
            first.list("subscription", Some("guest:quota")).await?.len(),
            1
        );
        assert_eq!(first.count("subscription", "guest:quota").await?, 1);
        assert_eq!(second.count("subscription", "guest:other").await?, 0);
        Ok(())
    }
}
