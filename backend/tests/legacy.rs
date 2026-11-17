use serde_json::{Value, json};
use subboost::{legacy::LegacyImport, store::Store};

const KEY: &str = "legacy-test-master-key";
const HASH: &str = "$2a$04$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy";
// Fixed nonces make these fixtures reproducible with Node's legacy HKDF/AES-GCM format.
const URLS: &str = "v2:010101010101010101010101:7bf71b064a23f7a5e0fbe228e69a13ce:27d2483f6f070c3fd0945fd7dd7701609f4d62757541c50a600c6d27d105d524976c20485876ba563bb4";
const NODES: &str = "v2:020202020202020202020202:0dd50918ad45f1332e0fa10fdd367f5a:321e887dc6a54f8fcf3e0b9a2987faf8d24ca6da990534777e067e5f5bd5762e075d833e15cda804fbc4a6c6ea6c2fb5b77c612a835e927a9d8275648ed0f086d8ea2eb45bea5d85efebf11bff3cfc7e1c44c625977bca9eb471e7f43431a44b10474bbe476088d77b5c799f4ea1b7e90d22d4dbca04ca93719db8f6d3fa9ec0";
const CONFIG: &str = "v2:030303030303030303030303:000718ce6de96204b1f1a2ddcded90b3:547c7b7bbd06174b894e6087e17ddbbdc6d4da8749e971aee94b3a0f699af6af751bab1706d0c3510a51aad871c988d37ff06a107d8a4c8ccaedd63a323e60b5f1a2467478f1a3230a77ac02436d6134fafc34e63f2f5421ccb649f990b90c2626552ec156df281722774651f060cf";
const INFO: &str = "v2:040404040404040404040404:514a380f4628bf04f76a0d3a586a8c4d:eddd7d5c1447db85571b45783f60fbd56a508106d4590c5940c2ce20a0131f8af00b3e0033b76197c859955c40573cac60bf356e6f64de5c5bde5fd3";

fn fixture() -> Value {
    json!({
        "format":"subboost-postgresql", "version":1,
        "admins":[{"id":"admin-old", "username":"operator", "passwordHash":HASH,
            "createdAt":"2024-01-01T00:00:00+00:00", "updatedAt":"2024-02-01T00:00:00+00:00",
            "lastLoginAt":"2024-03-01T00:00:00+00:00", "allowUnsafeSubscriptionSources":true}],
        "subscriptions":[{"id":"subscription-old", "ownerId":"admin-old", "name":"Old subscription",
            "token":"preserved-link-token", "isPrimary":true, "encryptedUrls":URLS, "encryptedNodes":NODES,
            "encryptedConfig":CONFIG, "encryptedSubscriptionInfo":INFO, "autoUpdateInterval":3600,
            "cacheExpiresAt":"2024-04-01T00:00:00+00:00", "lastAccessedAt":"2024-03-01T00:00:00+00:00",
            "lastUpdatedAt":"2024-03-02T00:00:00+00:00", "createdAt":"2024-01-01T00:00:00+00:00",
            "updatedAt":"2024-03-02T00:00:00+00:00"}],
        "templates":[{"id":"template-old", "ownerId":"admin-old", "name":"Old template", "description":"Saved description",
            "encryptedConfig":CONFIG, "createdAt":"2024-01-01T00:00:00+00:00", "updatedAt":"2024-03-02T00:00:00+00:00"}],
        "autoUpdateStates":[{"subscriptionId":"subscription-old", "externalFailureCount":2,
            "failureSourceState":"{\"source-1\":2}", "lastFailedAt":"2024-03-02T00:00:00+00:00",
            "lastAttemptedAt":"2024-03-02T00:00:00+00:00", "nodeQuotaFailureCount":1,
            "lastNodeQuotaExceededAt":"2024-02-01T00:00:00+00:00", "lastNodeQuotaActual":300,
            "lastNodeQuotaLimit":200, "disabledAt":"2024-03-02T00:00:00+00:00",
            "disabledReason":"external-failure", "disabledPreviousInterval":3600,
            "createdAt":"2024-01-01T00:00:00+00:00", "updatedAt":"2024-03-02T00:00:00+00:00"}],
        "sourceSettings":[{"ownerId":"admin-old", "allowUnsafeSubscriptionSources":true}]
    })
}

fn prepared(value: &Value) -> LegacyImport {
    LegacyImport::prepare(&value.to_string(), KEY).unwrap()
}

#[tokio::test]
async fn imports_deterministic_encrypted_export_without_plaintext_sqlite_documents() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("data.sqlite3"),
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    let export = fixture();
    let result = prepared(&export).import(&store).await.unwrap();
    assert_eq!(
        result,
        json!({"success":true,"admins":1,"subscriptions":1,"templates":1})
    );
    let (subscription, revision) = store
        .get("subscription", "admin-old", "subscription-old")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revision, 1);
    assert_eq!(subscription["token"], "preserved-link-token");
    assert_eq!(subscription["ownerId"], "admin-old");
    assert_eq!(
        subscription["urls"],
        json!(["https://secret.example/sub?key=private"])
    );
    assert_eq!(
        subscription["nodes"][0]["password"],
        "private-node-password"
    );
    assert_eq!(subscription["config"]["customField"]["preserved"], true);
    assert_eq!(subscription["subscriptionInfo"]["total"], 1000);
    assert_eq!(
        subscription["autoUpdateState"],
        export["autoUpdateStates"][0]
    );
    for field in [
        "isPrimary",
        "autoUpdateInterval",
        "cacheExpiresAt",
        "lastAccessedAt",
        "lastUpdatedAt",
        "createdAt",
        "updatedAt",
    ] {
        assert_eq!(subscription[field], export["subscriptions"][0][field]);
    }
    assert!(subscription.get("encryptedUrls").is_none());
    assert!(
        store
            .get("subscription", "local", "subscription-old")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .by_token("preserved-link-token")
            .await
            .unwrap()
            .unwrap()
            .0,
        "admin-old"
    );
    let (template, _) = store
        .get("template", "admin-old", "template-old")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(template["description"], "Saved description");
    assert_eq!(template["config"], subscription["config"]);
    let (profile, _) = store
        .get("admin-profile", "admin-old", "admin-old")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(profile["lastLoginAt"], export["admins"][0]["lastLoginAt"]);
    assert!(profile.get("passwordHash").is_none());
    assert_eq!(
        store
            .get("settings", "admin-old", "admin-old")
            .await
            .unwrap()
            .unwrap()
            .0["allowUnsafeSubscriptionSources"],
        true
    );
    store
        .run(|db| {
            let admin: (String, String, String) =
                db.query_row("SELECT id,username,password_hash FROM admin", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
            assert_eq!(
                admin,
                (
                    "admin-old".to_owned(),
                    "operator".to_owned(),
                    HASH.to_owned()
                )
            );
            let mut query = db.prepare("SELECT data FROM documents")?;
            let rows = query.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                let data = row?;
                assert!(serde_json::from_str::<Value>(&data).is_err());
                for secret in [
                    "secret.example",
                    "private-node-password",
                    "Saved description",
                    HASH,
                ] {
                    assert!(!data.contains(secret));
                }
            }
            let sessions: i64 = db.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
            assert_eq!(sessions, 0);
            Ok(())
        })
        .await
        .unwrap();
}

#[test]
fn rejects_wrong_keys_corrupt_ciphertext_and_broken_references_before_import() {
    let export = fixture();
    for key in ["", "  ", "wrong-master-key"] {
        assert!(LegacyImport::prepare(&export.to_string(), key).is_err());
    }
    for mutation in 0..6 {
        let mut export = fixture();
        match mutation {
            0 => export["subscriptions"][0]["encryptedUrls"] = "v2:00:00:00".into(),
            1 => export["subscriptions"][0]["encryptedUrls"] = "[\"plaintext\"]".into(),
            2 => export["subscriptions"][0]["ownerId"] = "unknown-owner".into(),
            3 => export["autoUpdateStates"][0]["subscriptionId"] = "unknown-subscription".into(),
            4 => {
                let admin = export["admins"][0].clone();
                export["admins"].as_array_mut().unwrap().push(admin);
            }
            _ => export["sourceSettings"][0]["allowUnsafeSubscriptionSources"] = false.into(),
        }
        assert!(LegacyImport::prepare(&export.to_string(), KEY).is_err());
    }
}

#[tokio::test]
async fn document_id_or_token_conflict_rolls_back_admin_and_every_imported_row() {
    for token_conflict in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(
            &directory.path().join("data.sqlite3"),
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        let (kind, existing) = if token_conflict {
            (
                "subscription",
                json!({"id":"existing", "token":"preserved-link-token", "name":"Do not overwrite"}),
            )
        } else {
            (
                "template",
                json!({"id":"template-old", "name":"Do not overwrite"}),
            )
        };
        store
            .insert(kind, "unrelated-owner", &existing, 1)
            .await
            .unwrap();
        assert!(prepared(&fixture()).import(&store).await.is_err());
        assert_eq!(
            store
                .get(kind, "unrelated-owner", existing["id"].as_str().unwrap())
                .await
                .unwrap()
                .unwrap()
                .0,
            existing
        );
        store
            .run(|db| {
                let admins: i64 = db.query_row("SELECT count(*) FROM admin", [], |r| r.get(0))?;
                let documents: i64 =
                    db.query_row("SELECT count(*) FROM documents", [], |r| r.get(0))?;
                assert_eq!(admins, 0);
                assert_eq!(documents, 1);
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn existing_administrator_is_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("data.sqlite3"),
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    store.run(|db| {
        db.execute("INSERT INTO admin(singleton,id,username,password_hash) VALUES(1,'existing','existing-user','existing-hash')", [])?;
        Ok(())
    }).await.unwrap();
    assert!(prepared(&fixture()).import(&store).await.is_err());
    store
        .run(|db| {
            let hash: String = db.query_row("SELECT password_hash FROM admin", [], |r| r.get(0))?;
            let documents: i64 =
                db.query_row("SELECT count(*) FROM documents", [], |r| r.get(0))?;
            assert_eq!(hash, "existing-hash");
            assert_eq!(documents, 0);
            Ok(())
        })
        .await
        .unwrap();
}

#[test]
fn cli_validates_before_creating_sqlite_and_prints_only_counts() {
    let directory = tempfile::tempdir().unwrap();
    let export = directory.path().join("legacy.json");
    let database = directory.path().join("data.sqlite3");
    std::fs::write(&export, fixture().to_string()).unwrap();
    for key in [None, Some("wrong-key")] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_subboost"));
        command
            .args(["--database"])
            .arg(&database)
            .arg("migrate-legacy")
            .arg(&export)
            .env_remove("ENCRYPTION_KEY");
        if let Some(key) = key {
            command.env("ENCRYPTION_KEY", key);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!database.exists());
        assert!(!database.with_extension("key").exists());
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_subboost"))
        .arg("--database")
        .arg(&database)
        .arg("migrate-legacy")
        .arg(&export)
        .env("ENCRYPTION_KEY", KEY)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"success":true,"admins":1,"subscriptions":1,"templates":1})
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("preserved-link-token"));
}

#[tokio::test]
async fn imports_optional_settings_and_nullable_metadata_without_inventing_old_state() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        &directory.path().join("data.sqlite3"),
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    let mut export = fixture();
    export.as_object_mut().unwrap().remove("sourceSettings");
    export["admins"][0]
        .as_object_mut()
        .unwrap()
        .remove("allowUnsafeSubscriptionSources");
    export["subscriptions"][0]["encryptedSubscriptionInfo"] = Value::Null;
    export["autoUpdateStates"] = json!([]);
    prepared(&export).import(&store).await.unwrap();
    let (subscription, _) = store
        .get("subscription", "admin-old", "subscription-old")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(subscription["subscriptionInfo"], json!({}));
    assert_eq!(subscription["autoUpdateState"]["externalFailureCount"], 0);
    assert_eq!(subscription["autoUpdateState"]["disabledAt"], Value::Null);
    assert!(
        store
            .get("settings", "admin-old", "admin-old")
            .await
            .unwrap()
            .is_none()
    );
}
