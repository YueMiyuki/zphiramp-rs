mod support;

use base64::Engine;
use serde_json::Value;
use zphira_http::{JwtAlgorithm, JwtService};

fn fixture() -> Value {
    let text = std::fs::read_to_string(support::fixtures().join("jwt.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn key(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap()
}

fn service() -> JwtService {
    JwtService::from_secret(key(fixture()["secret"].as_str().unwrap())).unwrap()
}

fn iat_millis(token: &str) -> i64 {
    let payload = token.split('.').nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    let claims: Value = serde_json::from_slice(&bytes).unwrap();
    claims["iat"].as_i64().unwrap() * 1000
}

#[test]
fn issued_tokens_are_byte_identical_to_jjwt() {
    let doc = fixture();
    let rows = doc["issued"].as_array().unwrap();
    assert_eq!(rows.len(), 45);
    for row in rows {
        let jwt = JwtService::from_secret(key(row["keyBase64"].as_str().unwrap())).unwrap();
        let token = jwt.issue_token_at(
            i32::try_from(row["userId"].as_i64().unwrap()).unwrap(),
            row["isAdmin"].as_bool().unwrap(),
            row["nowMillis"].as_i64().unwrap(),
        );
        assert_eq!(token, row["token"].as_str().unwrap(), "{row}");
    }
}

#[test]
fn java_issued_tokens_verify_in_rust() {
    let doc = fixture();
    for row in doc["issued"].as_array().unwrap() {
        let jwt = JwtService::from_secret(key(row["keyBase64"].as_str().unwrap())).unwrap();
        let token = row["token"].as_str().unwrap();
        let now = row["nowMillis"].as_i64().unwrap();
        let user = row["userId"].as_i64().unwrap();
        assert_eq!(
            jwt.parse_user_id_at(token, now).map(i64::from),
            Some(user),
            "{token}"
        );
        let other = JwtService::from_secret(vec![0xAA; 64]).unwrap();
        assert_eq!(other.parse_user_id_at(token, now), None);
    }
}

#[test]
fn key_length_selects_the_algorithm_like_hmac_sha_key_for() {
    let doc = fixture();
    for row in doc["keys"].as_array().unwrap() {
        let len = usize::try_from(row["length"].as_u64().unwrap()).unwrap();
        let rust = JwtService::from_secret((0..len).map(|i| i as u8).collect());
        match row["algorithm"].as_str() {
            Some(java) => {
                let expected = match java {
                    "HmacSHA256" => JwtAlgorithm::Hs256,
                    "HmacSHA384" => JwtAlgorithm::Hs384,
                    "HmacSHA512" => JwtAlgorithm::Hs512,
                    other => panic!("{other}"),
                };
                assert_eq!(rust.unwrap().algorithm(), expected, "length {len}");
            }
            None => assert!(
                rust.is_err(),
                "length {len} must be refused (WeakKeyException)"
            ),
        }
    }
}

#[test]
fn crafted_tokens_parse_like_jjwt() {
    let doc = fixture();
    let jwt = service();
    let parsed = doc["parsed"].as_array().unwrap();
    let now_row = parsed
        .iter()
        .find(|r| r["name"] == "issued now by JwtService")
        .unwrap();
    let now = iat_millis(now_row["token"].as_str().unwrap()) + 1000;
    let mut failures = Vec::new();
    for row in parsed {
        let token = row["token"].as_str().unwrap();
        let expected = row["userId"].as_i64().map(|v| i32::try_from(v).unwrap());
        let actual = jwt.parse_user_id_at(token, now);
        if actual != expected {
            failures.push(format!(
                "{}: Rust {actual:?}, jjwt {expected:?} ({})",
                row["name"], row["jjwt"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(parsed.len(), 78, "the recorded cases");
}

#[test]
fn the_rejections_the_brief_names() {
    let doc = fixture();
    let jwt = service();
    let token = |name: &str| -> String {
        doc["parsed"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let now = zphira_common::time::now_millis();
    assert_eq!(jwt.parse_user_id_at(&token("valid"), now), Some(7));
    for rejected in [
        "tampered payload",
        "tampered signature",
        "expired",
        "alg none, no signature",
        "alg none with signature",
        "wrong secret",
        "manual HS512 with the 32-byte secret",
    ] {
        assert_eq!(
            jwt.parse_user_id_at(&token(rejected), now),
            None,
            "{rejected}"
        );
    }
}

fn rust_token_cases() -> Vec<(String, i32, bool, i64)> {
    let valid_until_2100 = 4_102_444_800_000 - zphira_http::EXPIRATION_MILLIS;
    let mut cases = Vec::new();
    for (user, admin) in [
        (1, true),
        (2, false),
        (i32::MAX, false),
        (-5, true),
        (0, false),
    ] {
        cases.push((
            format!("valid {user} {admin}"),
            user,
            admin,
            valid_until_2100,
        ));
    }
    cases.push(("expired".to_owned(), 7, true, 1_600_000_000_000));
    cases.push((
        "fraction of a second".to_owned(),
        8,
        false,
        valid_until_2100 - 1,
    ));
    cases
}

#[test]
#[ignore = "fixture generator, run by hand before tests/java/gen.sh jwt"]
fn jwt_write_rust_tokens() {
    let jwt = service();
    let mut out = String::new();
    for (name, user, admin, now) in rust_token_cases() {
        out.push_str(&format!(
            "{name}\t{}\n",
            jwt.issue_token_at(user, admin, now)
        ));
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/http-gen");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rust-tokens.tsv"), out).unwrap();
}

#[test]
fn rust_issued_tokens_verified_by_jjwt() {
    let doc = fixture();
    let jwt = service();
    let rows = doc["rust"].as_array().expect(
        "jwt.json has no `rust` section: run jwt_write_rust_tokens, then tests/java/gen.sh jwt",
    );
    let cases = rust_token_cases();
    assert_eq!(rows.len(), cases.len());
    for ((name, user, admin, now), row) in cases.into_iter().zip(rows) {
        assert_eq!(row["name"].as_str().unwrap(), name);
        assert_eq!(
            row["token"].as_str().unwrap(),
            jwt.issue_token_at(user, admin, now)
        );
        let java = row["userId"].as_i64();
        if name == "expired" {
            assert_eq!(java, None, "jjwt rejects the expired token");
        } else {
            assert_eq!(java, Some(i64::from(user)), "jjwt accepts {name}");
        }
    }
}

#[test]
fn secret_file_is_created_0600_and_tightened() {
    let dir = tempfile::tempdir().unwrap();
    let jwt = JwtService::load(dir.path()).unwrap();
    let path = dir.path().join("jwt-secret");
    let text = std::fs::read_to_string(&path).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .unwrap();
    assert_eq!(bytes.len(), 32);
    let token = jwt.issue_token(3, false);
    assert_eq!(
        JwtService::from_secret(bytes)
            .unwrap()
            .parse_user_id(&token),
        Some(3)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let again = JwtService::load(dir.path()).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(again.parse_user_id(&token), Some(3));
    }
}

#[test]
fn bad_secret_files_fail_with_a_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jwt-secret");
    std::fs::write(&path, "not base64!").unwrap();
    let e = JwtService::load(dir.path()).unwrap_err().to_string();
    assert!(e.contains("not valid Base64"), "{e}");
    std::fs::write(&path, "AAECAwQFBgcICQ==").unwrap();
    let e = JwtService::load(dir.path()).unwrap_err().to_string();
    assert!(
        e.contains("80 bits") && e.contains("at least 256 bits"),
        "{e}"
    );
    std::fs::write(&path, "\n AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8 \n").unwrap();
    assert!(JwtService::load(dir.path()).is_ok());
}
