use std::collections::{HashMap, HashSet};

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::AeadInPlace};
use anyhow::{Context, Result, anyhow, ensure};
use hkdf::Hkdf;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;

use crate::store::Store;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Export {
    format: String,
    version: u32,
    admins: Vec<Value>,
    subscriptions: Vec<Value>,
    templates: Vec<Value>,
    auto_update_states: Vec<Value>,
    #[serde(default)]
    source_settings: Vec<Value>,
}

struct Document {
    kind: &'static str,
    id: String,
    owner: String,
    token: Option<String>,
    value: Value,
}

/// A fully decrypted and validated export. Preparation never opens or writes SQLite.
pub struct LegacyImport {
    admin: Option<(String, String, String)>,
    documents: Vec<Document>,
    subscriptions: usize,
    templates: usize,
}

impl LegacyImport {
    pub fn prepare(export: &str, master_key: &str) -> Result<Self> {
        ensure!(
            !master_key.trim().is_empty(),
            "Legacy ENCRYPTION_KEY is required"
        );
        let export: Export = serde_json::from_str(export).context("Invalid legacy export JSON")?;
        ensure!(
            export.format == "subboost-postgresql" && export.version == 1,
            "Unsupported legacy export format or version"
        );
        ensure!(
            export.admins.len() <= 1,
            "Legacy export contains multiple administrators; this installation supports one administrator"
        );
        let mut key = [0_u8; 32];
        Hkdf::<Sha256>::new(Some(b"subboost:encrypted-field:v2"), master_key.as_bytes())
            .expand(b"subboost:aes-256-gcm:v2", &mut key)
            .map_err(|_| anyhow!("Unable to derive legacy encryption key"))?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| anyhow!("Invalid legacy encryption key"))?;
        let mut documents =
            Vec::with_capacity(export.subscriptions.len() + export.templates.len() + 2);
        let mut admin = None;
        let mut owner = None;
        let mut settings = HashMap::new();
        for mut row in export.admins {
            let id = text(&row, "id")?.to_owned();
            ensure!(!id.starts_with("guest:"), "Invalid legacy administrator ID");
            let username = text(&row, "username")?.to_owned();
            let password_hash = text(&row, "passwordHash")?.to_owned();
            ensure!(
                password_hash.starts_with("$2a$")
                    || password_hash.starts_with("$2b$")
                    || password_hash.starts_with("$2y$")
                    || password_hash.starts_with("$argon2"),
                "Unsupported legacy administrator password hash"
            );
            if let Some(value) = row.get("allowUnsafeSubscriptionSources") {
                ensure!(value.is_boolean(), "Invalid legacy source setting");
                settings.insert(id.clone(), value.clone());
            }
            row.as_object_mut()
                .context("Invalid legacy administrator")?
                .remove("passwordHash");
            documents.push(Document {
                kind: "admin-profile",
                id: id.clone(),
                owner: id.clone(),
                token: None,
                value: row,
            });
            owner = Some(id.clone());
            admin = Some((id, username, password_hash));
        }
        let mut setting_owners = HashSet::new();
        for row in export.source_settings {
            let id = text(&row, "ownerId")?.to_owned();
            ensure!(
                owner.as_deref() == Some(id.as_str()),
                "Legacy source setting references an unknown owner"
            );
            ensure!(
                setting_owners.insert(id.clone()),
                "Duplicate legacy source setting"
            );
            let allowed = row
                .get("allowUnsafeSubscriptionSources")
                .filter(|v| v.is_boolean())
                .context("Invalid legacy source setting")?;
            if let Some(existing) = settings.get(&id) {
                ensure!(existing == allowed, "Conflicting legacy source settings");
            }
            settings.insert(id, allowed.clone());
        }
        for (id, allowed) in settings {
            documents.push(Document {
                kind: "settings",
                id: id.clone(),
                owner: id.clone(),
                token: None,
                value: json!({"id": id, "allowUnsafeSubscriptionSources": allowed}),
            });
        }
        let mut states = HashMap::with_capacity(export.auto_update_states.len());
        for row in export.auto_update_states {
            let id = text(&row, "subscriptionId")?.to_owned();
            ensure!(
                states.insert(id, row).is_none(),
                "Duplicate legacy refresh state"
            );
        }
        let subscriptions = export.subscriptions.len();
        let templates = export.templates.len();
        let mut ids = HashSet::new();
        let mut tokens = HashSet::new();
        for (kind, rows) in [
            ("subscription", export.subscriptions),
            ("template", export.templates),
        ] {
            for mut row in rows {
                let id = text(&row, "id")?.to_owned();
                let row_owner = text(&row, "ownerId")?.to_owned();
                ensure!(
                    owner.as_deref() == Some(row_owner.as_str()),
                    "Legacy document references an unknown owner"
                );
                ensure!(
                    ids.insert((kind, id.clone())),
                    "Duplicate legacy document ID"
                );
                text(&row, "name")?;
                let token = if kind == "subscription" {
                    let token = text(&row, "token")?.to_owned();
                    ensure!(
                        tokens.insert(token.clone()),
                        "Duplicate legacy subscription token"
                    );
                    Some(token)
                } else {
                    None
                };
                let fields: &[(&str, &str)] = if kind == "subscription" {
                    &[
                        ("encryptedUrls", "urls"),
                        ("encryptedNodes", "nodes"),
                        ("encryptedConfig", "config"),
                        ("encryptedSubscriptionInfo", "subscriptionInfo"),
                    ]
                } else {
                    &[("encryptedConfig", "config")]
                };
                for (encrypted, plain) in fields {
                    let value = match row.get(*encrypted) {
                        Some(Value::Null) | None if *plain == "subscriptionInfo" => json!({}),
                        Some(Value::String(value)) => {
                            decrypt_json(&cipher, value).with_context(|| {
                                format!(
                                    "Unable to decrypt legacy {kind} {plain}; check ENCRYPTION_KEY"
                                )
                            })?
                        }
                        _ => return Err(anyhow!("Missing legacy encrypted field {encrypted}")),
                    };
                    let valid = match *plain {
                        "urls" => value
                            .as_array()
                            .is_some_and(|values| values.iter().all(Value::is_string)),
                        "nodes" => value
                            .as_array()
                            .is_some_and(|values| values.iter().all(Value::is_object)),
                        _ => value.is_object(),
                    };
                    ensure!(valid, "Invalid legacy {plain} JSON shape");
                    let object = row.as_object_mut().context("Invalid legacy document")?;
                    object.remove(*encrypted);
                    object.insert((*plain).to_owned(), value);
                }
                if kind == "subscription" {
                    row["autoUpdateState"] = states.remove(&id).unwrap_or_else(|| json!({
                        "externalFailureCount":0, "failureSourceState":null, "lastFailedAt":null,
                        "lastAttemptedAt":null, "nodeQuotaFailureCount":0, "lastNodeQuotaExceededAt":null,
                        "lastNodeQuotaActual":null, "lastNodeQuotaLimit":null, "disabledAt":null,
                        "disabledReason":null, "disabledPreviousInterval":null
                    }));
                }
                documents.push(Document {
                    kind,
                    id,
                    owner: row_owner,
                    token,
                    value: row,
                });
            }
        }
        ensure!(
            states.is_empty(),
            "Legacy refresh state references an unknown subscription"
        );
        Ok(Self {
            admin,
            documents,
            subscriptions,
            templates,
        })
    }

    pub async fn import(self, store: &Store) -> Result<Value> {
        // Encrypt before entering the transaction; plaintext is never bound to SQLite.
        let mut records = Vec::with_capacity(self.documents.len());
        for document in self.documents {
            let data = store.encrypt(&document.value)?;
            records.push((
                document.kind,
                document.id,
                document.owner,
                document.token,
                data,
            ));
        }
        let admin_count = usize::from(self.admin.is_some());
        store.run(move |db| {
            let transaction = db.transaction()?;
            if let Some((id, username, password_hash)) = self.admin {
                transaction.execute("INSERT INTO admin(singleton,id,username,password_hash) VALUES(1,?1,?2,?3)", params![id, username, password_hash])
                    .context("Legacy administrator conflicts with existing data; nothing imported")?;
            }
            for (kind, id, owner, token, data) in records {
                transaction.execute("INSERT INTO documents(kind,id,owner,token,data) VALUES(?1,?2,?3,?4,?5)", params![kind, id, owner, token, data])
                    .context("Legacy document conflicts with existing data; nothing imported")?;
            }
            transaction.commit()?;
            Ok(())
        }).await?;
        Ok(
            json!({"success":true, "admins":admin_count, "subscriptions":self.subscriptions, "templates":self.templates}),
        )
    }
}

fn text<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Legacy {key} must be a nonempty string"))
}

fn decrypt_json(cipher: &Aes256Gcm, value: &str) -> Result<Value> {
    let mut parts = value.split(':');
    let (Some("v2"), Some(iv), Some(tag), Some(encrypted), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(anyhow!(
            "Unsupported legacy encrypted-field format (expected v2)"
        ));
    };
    let mut nonce = [0_u8; 12];
    unhex(iv, &mut nonce)?;
    let length = encrypted.len() / 2;
    let mut ciphertext = vec![0_u8; length + 16];
    unhex(encrypted, &mut ciphertext[..length])?;
    unhex(tag, &mut ciphertext[length..])?;
    cipher
        .decrypt_in_place(Nonce::from_slice(&nonce), b"", &mut ciphertext)
        .map_err(|_| anyhow!("Legacy encrypted-field authentication failed"))?;
    serde_json::from_slice(&ciphertext).context("Invalid legacy encrypted JSON")
}

fn unhex(value: &str, bytes: &mut [u8]) -> Result<()> {
    ensure!(
        value.len() == bytes.len() * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid legacy encrypted-field encoding"
    );
    for (pair, byte) in value.as_bytes().chunks_exact(2).zip(bytes) {
        let digit = |byte: u8| {
            if byte <= b'9' {
                byte - b'0'
            } else {
                byte.to_ascii_lowercase() - b'a' + 10
            }
        };
        *byte = (digit(pair[0]) << 4) | digit(pair[1]);
    }
    Ok(())
}
