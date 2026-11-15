mod support;

use serde_json::{Value, json};
use support::{Env, error_body};

const KINDS: [&str; 5] = ["admins", "pools", "points", "records", "submissions"];

async fn export(env: &Env, kind: &str) -> String {
    let r = env.get(&format!("/api/v1/data/{kind}/export")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    assert_eq!(
        r.header("Content-Disposition"),
        Some(format!("attachment; filename=\"{kind}.json\"").as_str())
    );
    r.text()
}

async fn import(env: &Env, kind: &str, query: &str, document: &str) -> Value {
    let r = env
        .call(
            "POST",
            &format!("/api/v1/data/{kind}/import{query}"),
            Some(&env.admin.clone()),
            Some(document),
        )
        .await;
    assert_eq!(r.status, 200, "{kind}: {}", r.text());
    r.json()
}

#[tokio::test]
async fn export_import_round_trip_per_kind() {
    let env = Env::new().await;
    let other = Env::new().await;
    for kind in KINDS {
        let document = export(&env, kind).await;
        let result = import(&env, kind, "?mode=replace", &document).await;
        assert_eq!(result["imported"], result["total"]);
        assert_eq!(
            export(&env, kind).await,
            document,
            "{kind}: replace with own export"
        );
        let result = import(&env, kind, "?mode=merge", &document).await;
        assert_eq!(result["imported"], result["total"]);
        assert_eq!(
            export(&env, kind).await,
            document,
            "{kind}: merge with own export"
        );
        import(&other, kind, "", &document).await;
        assert_eq!(export(&other, kind).await, document, "{kind}: moved");
    }
    for (kind, file) in [
        ("admins", "admins.json"),
        ("pools", "chart-pools.json"),
        ("points", "player-points.json"),
        ("records", "round-records.json"),
        ("submissions", "submissions.json"),
    ] {
        let on_disk = std::fs::read_to_string(other.data.join(file)).unwrap();
        assert_eq!(on_disk, export(&other, kind).await, "{kind}");
    }
}

#[tokio::test]
async fn backup_and_restore_round_trip_per_kind() {
    let env = Env::new().await;
    let replacements = [
        ("admins", "[1,9]"),
        (
            "pools",
            "{\"pools\":[{\"id\":7,\"default_flag\":true,\"chart_ids\":[1004]}]}",
        ),
        ("points", "{\"200\":{\"name\":\"Zed\",\"points\":1}}"),
        ("records", "[]"),
        ("submissions", "{\"submissions\":[]}"),
    ];
    for (kind, replacement) in replacements {
        let before = export(&env, kind).await;
        let result = import(&env, kind, "", replacement).await;
        let backup = result["backup"].as_str().unwrap().to_owned();
        let name = std::path::Path::new(&backup)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            std::path::Path::new(&backup),
            env.data.join("backups").join(&name),
            "{kind}: Java's path, under the data directory"
        );
        assert!(name.starts_with(&format!("{kind}-")) && name.ends_with(".json"));
        assert_eq!(name.len(), kind.len() + "-20261115-011815-585.json".len());
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            before,
            "{kind}: backup is the old state"
        );
        assert_ne!(export(&env, kind).await, before);

        let listing = env
            .get(&format!("/api/v1/data/{kind}/backups"))
            .await
            .json();
        assert_eq!(listing, json!({"ok": true, "backups": [name.clone()]}));

        let r = env
            .call(
                "POST",
                &format!("/api/v1/data/{kind}/restore"),
                Some(&env.admin.clone()),
                Some(&format!("{{\"backup\":\"{name}\"}}")),
            )
            .await;
        assert_eq!(r.status, 200, "{kind}: {}", r.text());
        assert_eq!(
            r.json().as_object().unwrap().keys().collect::<Vec<_>>(),
            ["ok", "imported"]
        );
        assert_eq!(export(&env, kind).await, before, "{kind}: restored");
        let listing = env
            .get(&format!("/api/v1/data/{kind}/backups"))
            .await
            .json();
        let names = listing["backups"].as_array().unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[1], json!(name));
    }
    std::fs::write(env.data.join("backups/notes.txt"), "x").unwrap();
    std::fs::write(env.data.join("backups/adminsX.json"), "x").unwrap();
    let listing = env.get("/api/v1/data/admins/backups").await.json();
    assert_eq!(listing["backups"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn restore_names_cannot_escape_the_backup_directory() {
    let env = Env::new().await;
    import(&env, "admins", "?mode=merge", "[5]").await;
    let name = env.backups("admins")[0].clone();
    std::fs::write(env.dir.path().join("admins-outside.json"), "[1,666]").unwrap();
    std::fs::write(env.data.join("admins-inside-data.json"), "[1,667]").unwrap();
    for backup in [
        "../admins.json".to_owned(),
        "../../admins-outside.json".to_owned(),
        "admins-/../../admins-outside.json".to_owned(),
        "admins-/../admins-inside-data.json".to_owned(),
        format!("/{}", env.dir.path().join("admins-outside.json").display()),
        "admins-/..".to_owned(),
        "admins-/.".to_owned(),
        "/".to_owned(),
        "admins-\u{0}.json".to_owned(),
    ] {
        let body = json!({"backup": backup}).to_string();
        let r = env
            .call(
                "POST",
                "/api/v1/data/admins/restore",
                Some(&env.admin.clone()),
                Some(&body),
            )
            .await;
        assert_eq!(r.json(), error_body("备份不存在"), "{backup:?}");
    }
    assert_eq!(
        env.get("/api/v1/admin").await.json()["admins"],
        json!([1, 5])
    );
    let r = env
        .call(
            "POST",
            "/api/v1/data/admins/restore",
            Some(&env.admin.clone()),
            Some(&json!({"backup": name}).to_string()),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(env.get("/api/v1/admin").await.json()["admins"], json!([1]));
}

#[tokio::test]
async fn refused_documents_leave_everything_unchanged() {
    let env = Env::new().await;
    let bad = [
        ("admins", "[\"x\"]"),
        ("pools", "{\"pools\":[{\"id\":1,\"category\":\"WRONG\"}]}"),
        ("points", "{\"abc\":{\"points\":1}}"),
        ("records", "[{\"id\":\"x\"}]"),
        ("submissions", "{\"submissions\":[{\"pool_id\":1}]}"),
    ];
    let snapshot = |env: &Env| -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        for entry in walk(&env.data) {
            files.push((entry.display().to_string(), std::fs::read(&entry).unwrap()));
        }
        files.sort();
        files
    };
    for (kind, document) in bad {
        let files = snapshot(&env);
        let exported = export(&env, kind).await;
        for query in ["", "?mode=merge"] {
            let r = env
                .call(
                    "POST",
                    &format!("/api/v1/data/{kind}/import{query}"),
                    Some(&env.admin.clone()),
                    Some(document),
                )
                .await;
            assert_eq!(r.status, 400, "{kind}{query}: {}", r.text());
        }
        assert_eq!(snapshot(&env), files, "{kind}: files changed");
        assert_eq!(export(&env, kind).await, exported, "{kind}: memory changed");
    }
    assert!(!env.data.join("backups").exists() || env.backups("admins").is_empty());
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn io_failures_are_500_with_the_java_prefix() {
    let env = Env::new().await;
    std::fs::create_dir_all(env.data.join("backups/admins-dir.json")).unwrap();
    let r = env
        .call(
            "POST",
            "/api/v1/data/admins/restore",
            Some(&env.admin.clone()),
            Some("{\"backup\":\"admins-dir.json\"}"),
        )
        .await;
    assert_eq!(r.status, 500);
    assert!(
        r.json()["reason"]
            .as_str()
            .unwrap()
            .starts_with("恢复失败："),
        "{}",
        r.text()
    );

    let env = Env::new().await;
    std::fs::write(env.data.join("backups"), "not a directory").unwrap();
    let r = env.get("/api/v1/data/admins/backups").await;
    assert_eq!(r.status, 500);
    assert_eq!(r.json(), error_body("无法列出备份"));
    let r = env
        .call(
            "POST",
            "/api/v1/data/admins/import?mode=merge",
            Some(&env.admin.clone()),
            Some("[5]"),
        )
        .await;
    assert_eq!(r.status, 500);
    assert!(
        r.json()["reason"]
            .as_str()
            .unwrap()
            .starts_with("导入失败："),
        "{}",
        r.text()
    );
    assert_eq!(env.get("/api/v1/admin").await.json()["admins"], json!([1]));
    for (kind, document, file) in [
        ("points", "{\"1\":{\"points\":3}}", "player-points.json"),
        ("records", "[]", "round-records.json"),
    ] {
        let before = (
            export(&env, kind).await,
            std::fs::read(env.data.join(file)).unwrap(),
        );
        let r = env
            .call(
                "POST",
                &format!("/api/v1/data/{kind}/import"),
                Some(&env.admin.clone()),
                Some(document),
            )
            .await;
        assert_eq!(r.status, 500, "{kind}: {}", r.text());
        assert!(
            r.json()["reason"]
                .as_str()
                .unwrap()
                .starts_with("导入失败："),
            "{}",
            r.text()
        );
        assert_eq!(
            (
                export(&env, kind).await,
                std::fs::read(env.data.join(file)).unwrap()
            ),
            before,
            "{kind}"
        );
    }
    let r = env
        .call(
            "POST",
            "/api/v1/data/pools/import?mode=merge",
            Some(&env.admin.clone()),
            Some("{\"pools\":[{\"id\":9}]}"),
        )
        .await;
    assert_eq!(r.status, 500);
    assert!(
        r.json()["reason"]
            .as_str()
            .unwrap()
            .starts_with("导入失败："),
        "{}",
        r.text()
    );
    assert_eq!(
        env.get("/api/v1/pool/list").await.json()["pools"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn backups_come_from_memory() {
    let env = Env::build(Default::default(), |dir| {
        std::fs::remove_file(dir.join("data/player-points.json")).unwrap();
        std::fs::remove_file(dir.join("data/round-records.json")).unwrap();
        std::fs::remove_file(dir.join("data/submissions.json")).unwrap();
    })
    .await;
    assert_eq!(export(&env, "points").await, "{}");
    assert_eq!(export(&env, "records").await, "[]");
    assert_eq!(export(&env, "submissions").await, "{\"submissions\":[]}");
    let result = import(&env, "points", "", "{\"1\":{\"points\":3}}").await;
    assert_eq!(result["backup"], json!(""));
    let result = import(&env, "submissions", "", "{\"submissions\":[]}").await;
    assert_eq!(result["backup"], json!(""));
    env.stores.records.append(zphira_ledger::RoundRecord {
        id: "r1-1-1".into(),
        room_id: "r1".into(),
        chart_id: 1,
        ..Default::default()
    });
    assert!(!env.data.join("round-records.json").exists());
    let result = import(&env, "records", "", "[]").await;
    let backup = result["backup"].as_str().unwrap();
    assert!(std::fs::read_to_string(backup).unwrap().contains("r1-1-1"));
}
