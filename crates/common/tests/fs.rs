use std::io::{self, Write};

use zphira_common::fs::{self, AtomicWriteOptions};
use zphira_common::gson;

fn temp_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

#[test]
fn read_missing_and_present() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.json");
    assert_eq!(fs::read_to_string_if_exists(&p).unwrap(), None);
    fs::write_string(dir.path().join("sub/dir/a.json"), "x").unwrap();
    assert_eq!(
        fs::read_to_string_if_exists(dir.path().join("sub/dir/a.json"))
            .unwrap()
            .as_deref(),
        Some("x")
    );
    std::fs::write(&p, [0xff, 0xfe]).unwrap();
    assert_eq!(
        fs::read_to_string_if_exists(&p).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn atomic_write_creates_and_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("data").join("player-points.json");
    fs::write_atomic(&p, "first").unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "first");
    fs::write_atomic(&p, "second, longer").unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "second, longer");
    assert!(temp_files(p.parent().unwrap()).is_empty());

    let mut m = std::collections::BTreeMap::new();
    m.insert(1, "a");
    fs::write_atomic_with(&p, &AtomicWriteOptions::new(), |w| {
        gson::to_writer_pretty(w, &m)
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\n  \"1\": \"a\"\n}");
}

#[test]
fn failure_leaves_target_and_no_temp() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("rooms.json");
    fs::write_atomic(&p, "old").unwrap();
    let err = fs::write_atomic_with(&p, &AtomicWriteOptions::new(), |w| {
        w.write_all(b"half")?;
        Err::<(), io::Error>(io::Error::other("disk full"))
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "disk full");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
    assert!(temp_files(dir.path()).is_empty());

    let r = fs::write_atomic_with(&p, &AtomicWriteOptions::new(), |w| {
        gson::to_writer_pretty(w, &f64::NAN)
    });
    assert_eq!(r.unwrap_err().kind(), gson::GsonErrorKind::Serialize);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
    assert!(temp_files(dir.path()).is_empty());

    let missing = dir.path().join("nope").join("x.json");
    assert!(
        fs::write_atomic_with(
            &missing,
            &AtomicWriteOptions::new().create_parents(false),
            |w| w.write_all(b"x")
        )
        .is_err()
    );
}

#[test]
fn stale_temp_files_are_removed() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("submissions.json");
    std::fs::write(dir.path().join(".submissions.json.123-0.tmp"), "junk").unwrap();
    std::fs::write(dir.path().join(".other.json.123-0.tmp"), "keep").unwrap();
    assert_eq!(fs::remove_stale_temp_files(&p).unwrap(), 1);
    assert_eq!(
        temp_files(dir.path()),
        vec![".other.json.123-0.tmp".to_owned()]
    );
    assert_eq!(
        fs::remove_stale_temp_files(dir.path().join("missing/x.json")).unwrap(),
        0
    );
}

#[cfg(unix)]
#[test]
fn permissions() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let dir = tempfile::tempdir().unwrap();

    let secret = dir.path().join("jwt-secret");
    fs::write_atomic_with(&secret, &AtomicWriteOptions::new().mode(0o600), |w| {
        w.write_all(b"k")
    })
    .unwrap();
    assert_eq!(mode(&secret), 0o600);

    let data = dir.path().join("admins.json");
    std::fs::write(&data, "[]").unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o640)).unwrap();
    fs::write_atomic(&data, "[1]").unwrap();
    assert_eq!(mode(&data), 0o640);

    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_mode(&data, 0o600).unwrap();
    assert_eq!(mode(&data), 0o600);

    let plain = dir.path().join("plain.json");
    std::fs::write(dir.path().join("probe"), "").unwrap();
    let default = mode(&dir.path().join("probe"));
    fs::write_atomic(&plain, "{}").unwrap();
    assert_eq!(mode(&plain), default);
}
