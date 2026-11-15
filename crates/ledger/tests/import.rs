mod support;

use std::path::Path;
use std::sync::Arc;

use support::ManualClock;
use zphira_ledger::{
    Admins, ImportError, ImportMode, ImportReport, PointAward, Points, RoundRecord, RoundRecords,
};

const T0: i64 = 1_700_000_000_000;

fn points(dir: &Path) -> Points {
    Points::load_with_clock(dir, ManualClock::new(T0).clock()).unwrap()
}

fn award(id: i32, name: &str, gained: i32) -> PointAward {
    PointAward {
        player_id: id,
        name: Some(name.to_owned()),
        gained,
    }
}

fn invalid(result: Result<ImportReport, ImportError>) -> String {
    match result {
        Err(ImportError::Invalid(message)) => message,
        other => panic!("expected a refused document, got {other:?}"),
    }
}

fn file(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

fn temp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

fn row(id: &str, room: &str, finished: i64) -> String {
    format!(
        "{{\"id\":\"{id}\",\"room_id\":\"{room}\",\"chart_id\":1,\"started_at\":{},\"finished_at\":{finished},\"results\":[]}}",
        finished - 1
    )
}

fn simple(id: &str, finished: i64) -> RoundRecord {
    RoundRecord {
        id: id.to_owned(),
        room_id: "room".to_owned(),
        finished_at: finished,
        ..Default::default()
    }
}

fn ids(records: &RoundRecords) -> String {
    records
        .query(100_000, 0, None, None)
        .iter()
        .map(|r| r.id.clone())
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn points_replace_and_merge_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points_batch(&[
        award(1, "Ann", 100),
        award(2, "Bo", 300),
        award(3, "Cy", 50),
    ]);

    let report = p
        .import(
            r#"{"3":{"name":"Cy2","points":400},"10":{"name":"Ten","points":1},"2":{"points":7}}"#,
            ImportMode::Merge,
        )
        .unwrap();
    assert_eq!((report.imported, report.total), (4, 4));
    let board: Vec<String> = p
        .ranking_snapshot(10)
        .iter()
        .map(|e| format!("{}:{:?}:{}", e.player_id, e.name, e.points))
        .collect();
    assert_eq!(
        board,
        vec![
            "3:Some(\"Cy2\"):400",
            "1:Some(\"Ann\"):100",
            "2:None:7",
            "10:Some(\"Ten\"):1"
        ]
    );

    let report = p
        .import(r#"{"8":{"name":"Eight","points":80}}"#, ImportMode::Replace)
        .unwrap();
    assert_eq!((report.imported, report.total), (1, 1));
    assert_eq!(p.len(), 1);
    assert_eq!(p.rank(1), 2, "unknown again");
    let report = p.import("{}", ImportMode::Replace).unwrap();
    assert_eq!((report.imported, report.total), (0, 0));
    assert!(p.is_empty());
}

#[test]
fn points_import_writes_the_file_in_the_java_layout_and_matches_memory() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.import(
        r#"{"20":{"name":"<b>","points":2},"3.0":{"points":9},"-1":{"name":"n","points":0}}"#,
        ImportMode::Replace,
    )
    .unwrap();
    let written = file(dir.path(), "player-points.json").unwrap();
    assert_eq!(
        written,
        "{\n  \"-1\": {\n    \"name\": \"n\",\n    \"points\": 0\n  },\n  \"3\": {\n    \"points\": 9\n  },\n  \"20\": {\n    \"name\": \"\\u003cb\\u003e\",\n    \"points\": 2\n  }\n}"
    );
    assert_eq!(p.export().unwrap(), written);
    std::fs::remove_file(dir.path().join("player-points.json")).unwrap();
    p.flush().unwrap();
    assert!(file(dir.path(), "player-points.json").is_none());
    std::fs::write(dir.path().join("player-points.json"), &written).unwrap();
    assert_eq!(points(dir.path()).export().unwrap(), written);
}

#[test]
fn points_document_level_checks_use_the_java_messages() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        assert_eq!(invalid(p.import("", mode)), "内容为空");
        assert_eq!(invalid(p.import(" \n\t ", mode)), "内容为空");
        assert_eq!(invalid(p.import("{", mode)), "不是有效的 JSON");
        assert_eq!(invalid(p.import("null", mode)), "不是有效的 JSON");
        assert_eq!(
            invalid(p.import("not-json", mode)),
            "JSON 顶层必须是对象或数组"
        );
        assert_eq!(invalid(p.import("5", mode)), "JSON 顶层必须是对象或数组");
        assert_eq!(
            invalid(p.import("[]", mode)),
            "玩家积分应为对象（用户 ID 到积分）"
        );
        assert_eq!(
            invalid(p.import(r#"[{"1":{"points":1}}]"#, mode)),
            "玩家积分应为对象（用户 ID 到积分）"
        );
    }
}

#[test]
fn points_bad_rows_are_refused_and_the_first_one_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points_batch(&[award(1, "Ann", 100)]);
    p.flush().unwrap();
    let before_file = file(dir.path(), "player-points.json").unwrap();
    let before_memory = p.export().unwrap();

    let cases: [(&str, &str); 13] = [
        (r#"{"1":{"points":1},"x":{"points":2},"3":5}"#, "第 2 项"),
        (r#"{"abc":{"points":2}}"#, "玩家 ID \"abc\""),
        (r#"{"2147483648":{"points":2}}"#, "必须是整数"),
        (r#"{"1":5}"#, "必须是对象"),
        (r#"{"1":null}"#, "必须是对象"),
        (r#"{"1":[]}"#, "必须是对象"),
        (r#"{"1":{}}"#, "缺少 points"),
        (r#"{"1":{"points":null}}"#, "缺少 points"),
        (r#"{"1":{"points":-5}}"#, "points 不能为负数"),
        (r#"{"1":{"points":"many"}}"#, "\"many\""),
        (r#"{"1":{"points":1.5}}"#, "points"),
        (r#"{"1":{"points":1,"name":{}}}"#, "name"),
        (r#"{"1":{"points":1},"1.0":{"points":2}}"#, "重复"),
    ];
    for (document, needle) in cases {
        for mode in [ImportMode::Replace, ImportMode::Merge] {
            let message = invalid(p.import(document, mode));
            assert!(message.starts_with("积分数据第 "), "{document}: {message}");
            assert!(message.contains(needle), "{document}: {message}");
        }
        assert_eq!(file(dir.path(), "player-points.json").unwrap(), before_file);
        assert_eq!(p.export().unwrap(), before_memory, "{document}");
    }
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn the_wrapper_body_of_the_api_doc_cannot_become_the_points_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "Ann", 5);
    p.flush().unwrap();
    let message = invalid(p.import(
        r#"{"kind":"points","content":"{\"1\":{\"points\":3}}"}"#,
        ImportMode::Replace,
    ));
    assert!(message.contains("\"kind\""), "{message}");
    assert_eq!(p.ranking_snapshot(5)[0].points, 5);
    assert!(Points::load(dir.path()).is_ok(), "the file still loads");
}

#[test]
fn points_replace_drops_unflushed_changes_and_merge_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "Ann", 100);
    p.flush().unwrap();
    p.add_points(2, "Bo", 50);

    p.import(r#"{"9":{"points":9}}"#, ImportMode::Merge)
        .unwrap();
    assert_eq!(p.len(), 3);
    assert!(
        file(dir.path(), "player-points.json")
            .unwrap()
            .contains("\"Bo\"")
    );

    p.add_points(4, "Di", 1);
    p.import(r#"{"7":{"points":7}}"#, ImportMode::Replace)
        .unwrap();
    assert_eq!(p.len(), 1, "the unflushed change is gone with the rest");
}

#[test]
fn points_export_is_memory_not_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    assert_eq!(p.export().unwrap(), "{}");
    p.add_points(1, "Ann", 100);
    assert!(file(dir.path(), "player-points.json").is_none());
    assert!(p.export().unwrap().contains("\"Ann\""));
}

#[test]
fn points_import_keeps_the_top_lines_cache_and_rebuilds_the_ranking() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let p = Points::load_with_clock(dir.path(), clock.clock()).unwrap();
    p.add_points(1, "Ann", 100);
    assert_eq!(p.top_ranking_lines(10), vec!["#1 Ann：100 分"]);
    p.import(r#"{"2":{"name":"Bo","points":5}}"#, ImportMode::Replace)
        .unwrap();
    assert_eq!(p.ranking_snapshot(10)[0].player_id, 2);
    assert_eq!(
        p.top_ranking_lines(10),
        vec!["#1 Ann：100 分"],
        "cached, as in Java"
    );
    clock.advance(60_001);
    assert_eq!(p.top_ranking_lines(10), vec!["#1 Bo：5 分"]);
}

#[test]
fn records_replace_and_merge_semantics_and_order() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    let doc = |rows: &[String]| format!("[{}]", rows.join(","));

    let report = records
        .import(
            &doc(&[row("i1", "x", 20), row("i2", "x", 40), row("i3", "y", 60)]),
            ImportMode::Replace,
        )
        .unwrap();
    assert_eq!((report.imported, report.total), (3, 3));
    assert_eq!(
        ids(&records),
        "i3,i2,i1",
        "the last row of the document is the newest"
    );

    let report = records
        .import(
            &doc(&[row("i2", "z", 400), row("i9", "x", 2)]),
            ImportMode::Merge,
        )
        .unwrap();
    assert_eq!((report.imported, report.total), (4, 4));
    assert_eq!(ids(&records), "i2,i3,i1,i9");
    assert_eq!(
        records.find("i2").unwrap().room_id,
        "z",
        "the document's row wins"
    );
    assert_eq!(records.query(10, 0, Some("z"), None).len(), 1);

    let report = records
        .import(&doc(&[row("i9", "x", 2)]), ImportMode::Replace)
        .unwrap();
    assert_eq!((report.imported, report.total), (1, 1));
    assert_eq!(ids(&records), "i9");
    let report = records.import("[]", ImportMode::Replace).unwrap();
    assert_eq!(report.total, 0);
    assert!(records.is_empty());
    let report = records
        .import(
            &doc(&[row("a", "r", 1), row("b", "r", 2)]),
            ImportMode::Merge,
        )
        .unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(ids(&records), "b,a");
}

#[test]
fn records_merge_orders_by_finished_at_and_ties_keep_document_then_held_order() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("h1", 5));
    records.append(simple("h2", 10));
    records
        .import(
            &format!("[{},{}]", row("d1", "r", 10), row("d2", "r", 5)),
            ImportMode::Merge,
        )
        .unwrap();
    assert_eq!(ids(&records), "h2,d1,h1,d2");
    assert_eq!(ids(&RoundRecords::load(dir.path()).unwrap()), "h2,d1,h1,d2");
    records
        .import(
            &format!("[{},{}]", row("old", "r", 1), row("new", "r", 99)),
            ImportMode::Merge,
        )
        .unwrap();
    assert_eq!(ids(&records), "new,h2,d1,h1,d2,old");
    records
        .import(
            &format!("[{},{}]", row("late", "r", 90), row("early", "r", 10)),
            ImportMode::Replace,
        )
        .unwrap();
    assert_eq!(ids(&records), "early,late");
}

#[test]
fn records_import_matches_what_a_restart_reads() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    records
        .import(
            &format!("[{},{}]", row("a", "r", 1), row("b", "r", 2)),
            ImportMode::Replace,
        )
        .unwrap();
    let written = file(dir.path(), "round-records.json").unwrap();
    assert_eq!(records.export().unwrap(), written);
    assert!(temp_files(dir.path()).is_empty());
    assert_eq!(ids(&RoundRecords::load(dir.path()).unwrap()), "b,a");
    records
        .import(&format!("[{}]", row("c", "r", 3)), ImportMode::Merge)
        .unwrap();
    assert_eq!(ids(&records), "c,b,a");
    assert_eq!(ids(&RoundRecords::load(dir.path()).unwrap()), "c,b,a");
}

#[test]
fn records_document_level_checks() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        assert_eq!(invalid(records.import("  ", mode)), "内容为空");
        assert_eq!(invalid(records.import("[1,", mode)), "不是有效的 JSON");
        assert_eq!(invalid(records.import("{}", mode)), "records 应为数组");
        assert_eq!(
            invalid(records.import(r#"{"1":{"points":1}}"#, mode)),
            "records 应为数组"
        );
        assert_eq!(
            invalid(records.import("true", mode)),
            "JSON 顶层必须是对象或数组"
        );
    }
}

#[test]
fn records_bad_rows_are_refused_and_the_first_one_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("keep", 1));
    records.flush().unwrap();
    let before_file = file(dir.path(), "round-records.json").unwrap();
    let before = ids(&records);

    let good = row("ok", "r", 5);
    let cases: Vec<(String, &str)> = vec![
        ("[1,2]".to_owned(), "第 1 项"),
        (format!("[{good},null]"), "第 2 项"),
        (
            format!("[{good},{{\"room_id\":\"r\",\"results\":[]}}]"),
            "缺少 id",
        ),
        (r#"[{"id":"a","results":[]}]"#.to_owned(), "缺少 room_id"),
        (r#"[{"id":"a","room_id":"r"}]"#.to_owned(), "缺少 results"),
        (
            r#"[{"id":"","room_id":"r","results":[]}]"#.to_owned(),
            "id 不能为空",
        ),
        (
            r#"[{"id":"a","room_id":"r","results":{}}]"#.to_owned(),
            "results 必须是数组",
        ),
        (
            r#"[{"id":"a","room_id":"r","chart_id":null,"results":[]}]"#.to_owned(),
            "chart_id 不能为 null",
        ),
        (
            r#"[{"id":"a","room_id":"r","chart_id":1.5,"results":[]}]"#.to_owned(),
            "chart_id",
        ),
        (
            r#"[{"id":"a","room_id":"r","results":[{"rank":1}]}]"#.to_owned(),
            "成绩第 1 项无效：缺少 player_id",
        ),
        (
            r#"[{"id":"a","room_id":"r","results":[{"player_id":1},null]}]"#.to_owned(),
            "成绩第 2 项无效：必须是对象",
        ),
        (
            r#"[{"id":"a","room_id":"r","results":[{"player_id":1,"score":null}]}]"#.to_owned(),
            "score 不能为 null",
        ),
        (
            r#"[{"id":"a","room_id":"r","results":[{"player_id":1,"accuracy":"fast"}]}]"#
                .to_owned(),
            "\"fast\"",
        ),
    ];
    for (document, needle) in &cases {
        for mode in [ImportMode::Replace, ImportMode::Merge] {
            let message = invalid(records.import(document, mode));
            assert!(message.starts_with("比赛记录第 "), "{document}: {message}");
            assert!(message.contains(needle), "{document}: {message}");
        }
        assert_eq!(ids(&records), before);
        assert_eq!(file(dir.path(), "round-records.json").unwrap(), before_file);
    }
    let message = invalid(records.import(
        r#"[{"id":"named","room_id":"r","results":[{"rank":1}]}]"#,
        ImportMode::Replace,
    ));
    assert!(message.contains("id \"named\""), "{message}");
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn records_with_the_same_id_twice_or_non_finite_numbers_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    let message = invalid(records.import(
        &format!("[{},{}]", row("same", "r", 1), row("same", "r", 2)),
        ImportMode::Replace,
    ));
    assert!(
        message.contains("第 2 项") && message.contains("重复"),
        "{message}"
    );
    let message = invalid(records.import(
        r#"[{"id":"a","room_id":"r","results":[{"player_id":1,"accuracy":NaN}]}]"#,
        ImportMode::Replace,
    ));
    assert!(message.contains("accuracy"), "{message}");
    let message = invalid(records.import(
        r#"[{"id":"a","room_id":"r","results":[{"player_id":1,"std":Infinity}]}]"#,
        ImportMode::Replace,
    ));
    assert!(message.contains("std"), "{message}");
    assert!(records.is_empty());
}

#[test]
fn records_import_accepts_a_full_java_export_and_lenient_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    let java = std::fs::read_to_string(support::fixture_path("records_java.json")).unwrap();
    let report = records.import(&java, ImportMode::Replace).unwrap();
    assert_eq!(report.total, 5);
    assert_eq!(records.export().unwrap(), java, "export is the Java layout");

    records
        .import(
            r#"[{"id":"a","room_id":7,"chart_id":"12","started_at":5.0,"results":[{"player_id":"3","accuracy":"0.5","std":1}]}]"#,
            ImportMode::Replace,
        )
        .unwrap();
    let a = records.find("a").unwrap();
    assert_eq!((a.room_id.as_str(), a.chart_id, a.started_at), ("7", 12, 5));
    assert_eq!(a.results[0].player_id, 3);
    assert_eq!((a.results[0].accuracy, a.results[0].std), (0.5, 1.0));
}

#[test]
fn records_import_applies_the_cap_to_the_newest_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(dir.path()).unwrap();
    let rows: Vec<String> = (0..1003)
        .map(|i| row(&format!("m{i}"), "m", i + 1))
        .collect();

    let report = records
        .import(&format!("[{}]", rows.join(",")), ImportMode::Replace)
        .unwrap();
    assert_eq!((report.imported, report.total), (1000, 1000));
    assert_eq!(records.len(), 1000);
    assert_eq!(records.query(1, 0, None, None)[0].id, "m1002");
    assert_eq!(records.query(1, 999, None, None)[0].id, "m3");
    assert!(records.find("m2").is_none());
    let file_rows = records.export().unwrap().matches("\"id\"").count();
    assert_eq!(file_rows, 1000, "the file is capped too");
    records.append(simple("new", 5000));
    assert_eq!(records.len(), 1000);
    assert_eq!(records.query(1, 999, None, None)[0].id, "m4");

    let fresh = tempfile::tempdir().unwrap();
    let records = RoundRecords::load(fresh.path()).unwrap();
    for i in 0..600 {
        records.append(simple(&format!("held{i}"), 1000 + i));
    }
    let incoming: Vec<String> = (0..600)
        .map(|i| row(&format!("doc{i}"), "d", 1300 + i))
        .collect();
    let report = records
        .import(&format!("[{}]", incoming.join(",")), ImportMode::Merge)
        .unwrap();
    assert_eq!((report.imported, report.total), (1000, 1000));
    let newest = records.query(1, 0, None, None)[0].id.clone();
    let oldest = records.query(1, 999, None, None)[0].id.clone();
    assert_eq!(newest, "doc599");
    assert_eq!(oldest, "held200", "the 200 oldest held rounds were dropped");
    assert_eq!(RoundRecords::load(fresh.path()).unwrap().len(), 1000);
}

#[test]
fn admins_replace_and_merge_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let admins = Admins::load(dir.path()).unwrap();
    admins.import("[1,2,3]", ImportMode::Replace).unwrap();
    let report = admins.import("[3,4]", ImportMode::Merge).unwrap();
    assert_eq!((report.imported, report.total), (4, 4));
    assert_eq!(admins.list(), vec![1, 2, 3, 4]);
    let report = admins.import("[7]", ImportMode::Replace).unwrap();
    assert_eq!((report.imported, report.total), (1, 1));
    assert_eq!(admins.list(), vec![7]);
    assert_eq!(file(dir.path(), "admins.json").unwrap(), "[\n  7\n]");
    admins
        .import(r#"["9", 8.0, 10]"#, ImportMode::Replace)
        .unwrap();
    assert_eq!(admins.list(), vec![8, 9, 10]);
    let report = admins.import("[]", ImportMode::Merge).unwrap();
    assert_eq!(report.total, 3);
    let report = admins.import("[2,1]", ImportMode::Merge).unwrap();
    assert_eq!(report.total, 5);
}

#[test]
fn admins_import_never_leaves_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let admins = Admins::load(dir.path()).unwrap();
    admins.import("[1,2]", ImportMode::Replace).unwrap();
    let message = invalid(admins.import("[]", ImportMode::Replace));
    assert_eq!(message, "至少保留一名管理员");
    assert_eq!(admins.list(), vec![1, 2]);
    assert_eq!(file(dir.path(), "admins.json").unwrap(), "[\n  1,\n  2\n]");
    let empty_dir = tempfile::tempdir().unwrap();
    let empty = Admins::load(empty_dir.path()).unwrap();
    assert!(empty.list().is_empty());
    let message = invalid(empty.import("[]", ImportMode::Merge));
    assert_eq!(message, "至少保留一名管理员");
    let message = invalid(empty.import("[]", ImportMode::Replace));
    assert_eq!(message, "至少保留一名管理员");
    assert_eq!(file(empty_dir.path(), "admins.json").unwrap(), "[]");
    assert_eq!(empty.import("[7]", ImportMode::Merge).unwrap().total, 1);
}

#[test]
fn admins_wrong_documents_are_refused_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let admins = Admins::load(dir.path()).unwrap();
    admins.add(1).unwrap();
    admins.add(2).unwrap();
    let before = file(dir.path(), "admins.json").unwrap();

    for mode in [ImportMode::Replace, ImportMode::Merge] {
        assert_eq!(invalid(admins.import("", mode)), "内容为空");
        assert_eq!(invalid(admins.import("[1,", mode)), "不是有效的 JSON");
        assert_eq!(invalid(admins.import("{}", mode)), "admins 应为数组");
        assert_eq!(
            invalid(admins.import(r#"{"1":2}"#, mode)),
            "admins 应为数组"
        );
        assert_eq!(
            invalid(admins.import("7", mode)),
            "JSON 顶层必须是对象或数组"
        );
        let message = invalid(admins.import(r#"[{"id":"r1-1-1","room_id":"r1"}]"#, mode));
        assert!(message.contains("第 1 项"), "{message}");
        let message = invalid(admins.import("[3, null]", mode));
        assert!(message.contains("第 2 项"), "{message}");
        for bad in ["[true]", "[1.5]", r#"["x"]"#, "[[1]]", "[{}]"] {
            let message = invalid(admins.import(bad, mode));
            assert!(
                message.starts_with("管理员名单第 1 项无效"),
                "{bad}: {message}"
            );
        }
        let message = invalid(admins.import("[5, 6, 5]", mode));
        assert!(
            message.contains("第 3 项") && message.contains("重复"),
            "{message}"
        );
    }
    assert_eq!(admins.list(), vec![1, 2]);
    assert_eq!(file(dir.path(), "admins.json").unwrap(), before);
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn admins_import_is_visible_to_the_hot_reload_without_a_second_read() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let admins = Admins::load_with_clock(dir.path(), clock.clock()).unwrap();
    admins.import("[4,5]", ImportMode::Replace).unwrap();
    let (list, logs) = support::capture_logs(|| {
        clock.advance(5000);
        admins.list()
    });
    assert_eq!(list, vec![4, 5]);
    assert!(
        logs.is_empty(),
        "our own write is not seen as an outside edit: {logs:?}"
    );
}

#[test]
fn a_failed_write_changes_nothing_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();

    let p = points(dir.path());
    p.add_points(1, "Ann", 10);
    // A directory where the file goes: the temp file is written and the rename fails.
    std::fs::create_dir(dir.path().join("player-points.json")).unwrap();
    let err = p
        .import(r#"{"2":{"points":2}}"#, ImportMode::Replace)
        .unwrap_err();
    assert!(matches!(err, ImportError::Write(_)), "{err:?}");
    assert!(!err.is_invalid());
    assert_eq!(p.ranking_snapshot(5)[0].player_id, 1, "memory is as it was");

    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(simple("keep", 1));
    std::fs::create_dir(dir.path().join("round-records.json")).unwrap();
    let err = records
        .import(&format!("[{}]", row("x", "r", 1)), ImportMode::Replace)
        .unwrap_err();
    assert!(matches!(err, ImportError::Write(_)));
    assert_eq!(ids(&records), "keep");

    let admins = Admins::load(dir.path()).unwrap();
    admins.add(1).unwrap();
    std::fs::remove_file(dir.path().join("admins.json")).unwrap();
    std::fs::create_dir(dir.path().join("admins.json")).unwrap();
    let err = admins.import("[9]", ImportMode::Replace).unwrap_err();
    assert!(matches!(err, ImportError::Write(_)));
    assert_eq!(admins.list(), vec![1]);

    assert!(temp_files(dir.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn a_failed_write_keeps_the_previous_file_loadable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "Ann", 10);
    p.flush().unwrap();
    let before = file(dir.path(), "player-points.json").unwrap();

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that ignores permissions (root) cannot be made to fail this way.
    let enforced = std::fs::File::create(dir.path().join("probe")).is_err();
    let result = p.import(r#"{"2":{"points":2}}"#, ImportMode::Replace);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    if !enforced {
        let _ = std::fs::remove_file(dir.path().join("probe"));
        return;
    }
    assert!(matches!(result, Err(ImportError::Write(_))), "{result:?}");
    assert_eq!(file(dir.path(), "player-points.json").unwrap(), before);
    assert_eq!(p.len(), 1);
    assert!(Points::load(dir.path()).is_ok());
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn a_merge_import_does_not_lose_rounds_appended_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let records = Arc::new(RoundRecords::load(dir.path()).unwrap());
    let appender = {
        let records = Arc::clone(&records);
        std::thread::spawn(move || {
            for i in 0..400 {
                records.append(simple(&format!("live{i}"), i));
            }
        })
    };
    for round in 0..15 {
        records
            .import(
                &format!("[{}]", row(&format!("doc{}", round % 3), "doc", 1)),
                ImportMode::Merge,
            )
            .unwrap();
    }
    appender.join().unwrap();
    records.flush().unwrap();
    assert_eq!(records.len(), 403);
    assert!((0..400).all(|i| records.find(&format!("live{i}")).is_some()));
    assert_eq!(
        file(dir.path(), "round-records.json").unwrap(),
        records.export().unwrap()
    );
}

#[test]
fn a_merge_import_does_not_lose_points_awarded_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    let awarder = {
        let p = p.clone();
        std::thread::spawn(move || {
            for i in 0..400 {
                p.add_points(1000 + i, "live", 1);
            }
        })
    };
    for _ in 0..15 {
        p.import(r#"{"1":{"name":"doc","points":5}}"#, ImportMode::Merge)
            .unwrap();
    }
    awarder.join().unwrap();
    p.flush().unwrap();
    assert_eq!(p.len(), 401);
    assert_eq!(
        file(dir.path(), "player-points.json").unwrap(),
        p.export().unwrap()
    );
}
