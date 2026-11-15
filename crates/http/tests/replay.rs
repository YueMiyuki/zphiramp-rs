mod support;

use std::collections::HashMap;

use serde_json::{Value, json};
use support::replay::{self, Exchange};
use support::{Env, Reply, error_body};
use zphira_common::time::JavaOffsetDateTime;

type Patch = fn(&mut Value);
type Check = fn(&Reply) -> Result<(), String>;

enum Expect {
    Json(u16, Value, &'static str),
    Patch(Patch, &'static str),
    Check(Check, &'static str),
    Response(Patch, &'static str),
    LogOnly(&'static str),
}

impl Expect {
    fn reason(&self) -> &'static str {
        match self {
            Expect::Json(_, _, reason)
            | Expect::Patch(_, reason)
            | Expect::Check(_, reason)
            | Expect::Response(_, reason)
            | Expect::LogOnly(reason) => reason,
        }
    }
}

const BAD_FORMAT: &str = "请检查是否传入了错误的格式";

fn check(exchange: &Exchange, rust: &Reply, expect: Option<&Expect>) -> Vec<String> {
    let method = exchange.request["method"].as_str().unwrap();
    match expect {
        None | Some(Expect::LogOnly(_)) => replay::compare(method, &exchange.response, rust),
        Some(Expect::Patch(patch, _)) => {
            replay::compare_patched(method, &exchange.response, rust, patch)
        }
        Some(Expect::Json(status, body, _)) => {
            let mut out = Vec::new();
            if rust.status.as_u16() != *status {
                out.push(format!(
                    "status {} (expected {status})",
                    rust.status.as_u16()
                ));
            }
            let mut actual = rust.json();
            replay::normalize(&mut actual);
            if &actual != body {
                out.push(format!("body {actual} (expected {body})"));
            }
            if replay::header(&exchange.response, "Access-Control-Allow-Origin").is_some()
                != rust.header("Access-Control-Allow-Origin").is_some()
            {
                out.push("CORS headers".to_owned());
            }
            out
        }
        Some(Expect::Check(check, _)) => check(rust).err().into_iter().collect(),
        Some(Expect::Response(edit, _)) => {
            let mut java = exchange.response.clone();
            edit(&mut java);
            let mut out = replay::compare(method, &java, rust);
            if method == "HEAD" {
                let length = replay::header(&java, "Content-Length");
                if length != rust.header("Content-Length") {
                    out.push(format!(
                        "Content-Length {length:?} vs {:?}",
                        rust.header("Content-Length")
                    ));
                }
                if !rust.body.is_empty() {
                    out.push(format!("HEAD body {:?}", rust.text()));
                }
            }
            out
        }
    }
}

fn set_header(response: &mut Value, name: &str, value: &str) {
    let headers = response["headers"].as_array_mut().unwrap();
    headers.retain(|h| !h[0].as_str().unwrap().eq_ignore_ascii_case(name));
    headers.push(json!([name, value]));
}

fn head(response: &mut Value, status: u16, content_type: &str, length: usize) {
    response["status"] = json!(status);
    set_header(response, "Content-Type", content_type);
    set_header(response, "Content-Length", &length.to_string());
}

async fn run(scenario: &str, expected: Vec<(&'static str, Expect)>) {
    run_in(Env::new().await, scenario, expected).await;
}

async fn run_in(env: Env, scenario: &str, expected: Vec<(&'static str, Expect)>) {
    let expected: HashMap<&'static str, Expect> = expected.into_iter().collect();
    let (captured, _guard) = replay::capture();
    let exchanges = replay::load(scenario);
    let mut failures = Vec::new();
    for exchange in &exchanges {
        let expect = expected.get(exchange.name.as_str());
        let Some(request) = replay::build_request(&env, &exchange.request) else {
            failures.push(format!(
                "{}: request cannot be built in-process",
                exchange.name
            ));
            continue;
        };
        captured.take();
        let rust = env.send(request).await;
        let logs = captured.take();
        let mut diffs = check(exchange, &rust, expect);
        if expect.is_none() {
            let java_logs = replay::log_messages(&exchange.log);
            let rust_logs = replay::log_messages(&replay::relative_logs(&logs, &env.data));
            if java_logs != rust_logs {
                diffs.push(format!("log {java_logs:?} vs {rust_logs:?}"));
            }
        }
        if !diffs.is_empty() && std::env::var_os("REPLAY_DUMP").is_some() {
            eprintln!("=== {} -> {} {}", exchange.name, rust.status, rust.text());
        }
        if !diffs.is_empty() {
            let why = expect.map_or(String::new(), |e| {
                format!(" (expected because: {})", e.reason())
            });
            failures.push(format!("{}{why}: {}", exchange.name, diffs.join("; ")));
        }
    }
    for name in expected.keys() {
        assert!(
            exchanges.iter().any(|e| e.name == *name),
            "{scenario}: the expectations name an unknown exchange {name:?}"
        );
    }
    assert!(
        failures.is_empty(),
        "{scenario}: {} exchange(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const SHORT_CACHE: &str = "public, max-age=300";

fn short_cache(response: &mut Value) {
    set_header(response, "Cache-Control", SHORT_CACHE);
}

fn static_answer(
    r: &Reply,
    status: u16,
    content_type: &str,
    cache_control: Option<&str>,
    body: &str,
) -> Result<(), String> {
    let actual = (
        r.status.as_u16(),
        r.header("Content-Type"),
        r.header("Cache-Control"),
        r.text(),
    );
    if actual == (status, Some(content_type), cache_control, body.to_owned()) {
        Ok(())
    } else {
        Err(format!("{actual:?}"))
    }
}

fn err(reason: &str) -> Value {
    error_body(reason)
}

fn submissions(reply: &Reply) -> Vec<(i64, String, i64)> {
    reply.json()["submissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["chartId"].as_i64().unwrap(),
                s["status"].as_str().unwrap().to_owned(),
                s["submitterCount"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn expect_submissions(reply: &Reply, expected: &[(i64, &str, i64)]) -> Result<(), String> {
    let actual = submissions(reply);
    let expected: Vec<(i64, String, i64)> = expected
        .iter()
        .map(|(c, s, n)| (*c, (*s).to_owned(), *n))
        .collect();
    if reply.status.as_u16() == 200 && actual == expected {
        Ok(())
    } else {
        Err(format!("submissions {actual:?}, expected {expected:?}"))
    }
}

#[tokio::test]
async fn framework() {
    run(
        "framework",
        vec![
            (
                "body: lower-case enum",
                Expect::Json(
                    400,
                    err("该类别不支持规则生成"),
                    "A-26: enum names match case-insensitively, so `manual` is MANUAL",
                ),
            ),
            (
                "body: unknown enum",
                Expect::Json(
                    400,
                    err("未知类别：BOGUS"),
                    "A-26: an unknown enum name is refused (Java: null, then the format error)",
                ),
            ),
            (
                "body: enum number",
                Expect::Json(400, err("未知类别：5"), "A-26"),
            ),
            (
                "body: latin1 charset",
                Expect::Json(
                    400,
                    err("未知类别：MANÉ"),
                    "A-26 (the body is decoded as ISO-8859-1, as in Java)",
                ),
            ),
            (
                "body: invalid utf8",
                Expect::Json(
                    400,
                    err("未知类别：\u{fffd}("),
                    "A-26 (malformed UTF-8 becomes U+FFFD, as in Java)",
                ),
            ),
            (
                "body: bogus charset",
                Expect::Json(
                    400,
                    err(BAD_FORMAT),
                    "A-26: an unsupported charset is a malformed request (Java: 500)",
                ),
            ),
            (
                "body: 1000001 bytes, import",
                Expect::Check(
                    |r| {
                        if r.status.as_u16() == 200 && r.json()["imported"] == json!(1) {
                            Ok(())
                        } else {
                            Err(r.text())
                        }
                    },
                    "A-10: the import route takes up to 64 MiB",
                ),
            ),
            (
                "query: tag duplicate",
                Expect::Json(
                    400,
                    err("参数格式错误：tag"),
                    "A-26: a repeated tag is a client mistake (Java: 500)",
                ),
            ),
            ("login trailing slash", Expect::Check(|r| {
                let v = r.json();
                if r.status.as_u16() == 200 && v["userId"] == json!(1) && v["token"].is_string() { Ok(()) } else { Err(r.text()) }
            }, "N-18: the login route is exempt from authentication in both spellings (Java: 401)")),
            ("static: index.html", Expect::Check(|r| static_answer(r, 200, "text/html;charset=utf-8", None, "<!doctype html><title>root</title>\n"),
                "N-21: an .html file is served without the immutable cache header")),
            ("static: space raw-encoded", Expect::Check(|r| static_answer(r, 200, "text/plain", Some(SHORT_CACHE), "space\n"),
                "N-21: decoded once when the raw path names no file (Java: 404); O-08")),
            ("static: utf8 encoded", Expect::Check(|r| static_answer(r, 200, "text/plain", Some(SHORT_CACHE), "chinese\n"),
                "N-21 (Java: 404); O-08")),
            ("static: css", Expect::Response(short_cache,
                "O-08: only files under _next/static/ are immutable for a year (Java: every file but .html)")),
            ("static: jpg", Expect::Response(short_cache, "O-08")),
            ("static: json", Expect::Response(short_cache, "O-08")),
            ("static: webmanifest", Expect::Response(short_cache, "O-08")),
            ("static: txt", Expect::Response(short_cache, "O-08")),
            ("static: no extension", Expect::Response(short_cache, "O-08")),
            ("static: upper-case extension", Expect::Response(short_cache, "O-08")),
            ("static: nested", Expect::Response(short_cache, "O-08")),
            ("static: query string", Expect::Response(short_cache, "O-08")),
            ("static: dot segment inside", Expect::Response(short_cache, "O-08")),
            ("method override", Expect::Response(|r| {
                r["status"] = json!(404);
                set_header(r, "Content-Type", "text/plain");
                r["body"] = json!("Endpoint POST /api/v1/pool/list not found");
            }, "O-05: X-HTTP-Method-Override is ignored (Java: honoured)")),
            ("method override unknown", Expect::Response(|r| r["body"] = json!("Endpoint POST /api/v1/pool/list not found"),
                "O-05")),
            ("HEAD api auth", Expect::Response(|r| head(r, 200, "application/json", 481),
                "O-05: HEAD answers what GET answers, without the body (Java: 200 text/plain without running anything)")),
            ("HEAD api player", Expect::Response(|r| head(r, 403, "application/json", 36), "O-05")),
            ("HEAD unknown api", Expect::Response(|r| head(r, 404, "text/plain", 9), "O-05")),
            ("HEAD delete-only route", Expect::Response(|r| head(r, 404, "text/plain", 9), "O-05")),
            ("HEAD root", Expect::Response(|r| head(r, 200, "text/html;charset=utf-8", 35), "O-05")),
            ("HEAD static file", Expect::Response(|r| {
                head(r, 200, "text/css", 16);
                short_cache(r);
            }, "O-05, O-08")),
            ("HEAD missing file", Expect::Response(|r| head(r, 404, "text/plain", 9), "O-05")),
            ("HEAD login", Expect::Response(|r| head(r, 404, "text/plain", 9), "O-05")),
            ("static: traversal encoded slash", Expect::Check(|r| static_answer(r, 400, "text/plain", None, "Bad Request"),
                "N-21: the decoded path `../secret.txt` leaves the directory (Java: 404 for the raw name)")),
            ("static: traversal backslash", Expect::Check(|r| static_answer(r, 400, "text/plain", None, "Bad Request"),
                "N-21: a decoded backslash is refused (Java: 404 for the raw name)")),
        ],
    )
    .await;
}

fn pools_after_edits() -> Value {
    json!({"ok":true,"pools":[
        {"id":10,"favoriteId":9,"default":true,"chartIds":[1000,1002,1006],"category":"CONFIGURED","sizeLimit":null,"roundsPerStay":5,"order":0,"submissionOpen":true},
        {"id":0,"favoriteId":null,"default":true,"chartIds":[1000,1002],"category":"REGULAR","sizeLimit":15,"roundsPerStay":3,"order":10,"submissionOpen":false},
        {"id":2,"favoriteId":null,"default":false,"chartIds":[1003],"category":"MANUAL","sizeLimit":null,"roundsPerStay":null,"order":30,"submissionOpen":true},
        {"id":12,"favoriteId":null,"default":false,"chartIds":[],"category":"REGULAR","sizeLimit":null,"roundsPerStay":null,"order":50,"submissionOpen":false},
        {"id":1,"favoriteId":42,"default":false,"chartIds":[1001],"category":"CONFIGURED","sizeLimit":null,"roundsPerStay":null,"order":1000000000,"submissionOpen":false},
        {"id":13,"favoriteId":null,"default":false,"chartIds":[],"category":"CONFIGURED","sizeLimit":10,"roundsPerStay":null,"order":1000000010,"submissionOpen":false},
        {"id":14,"favoriteId":null,"default":false,"chartIds":[],"category":"CONFIGURED","sizeLimit":10,"roundsPerStay":null,"order":1000000020,"submissionOpen":false}
    ]})
}

#[tokio::test]
async fn pools() {
    let mut list_final = pools_after_edits();
    let pools = list_final["pools"].as_array_mut().unwrap();
    pools.retain(|p| p["id"] != json!(0) && p["id"] != json!(1));
    run(
        "pools",
        vec![
            ("create: unknown category", Expect::Json(400, err("未知类别：bogus"), "A-26 (Java created a MANUAL pool)")),
            ("create: chart ids with null", Expect::Json(400, err(BAD_FORMAT), "A-26: a null chart id is a malformed body (Java answered the NullPointerException text)")),
            ("list after create", Expect::Patch(|v| {
                let pools = v["pools"].as_array_mut().unwrap();
                pools.retain(|p| p["id"] != json!(13));
                for p in pools.iter_mut() {
                    if p["id"] == json!(10) { p["favoriteId"] = json!(77); }
                    if p["id"] == json!(12) { p["category"] = json!("REGULAR"); }
                }
            }, "A-25 favoriteId honoured on create; A-26 `regular` is REGULAR and `bogus` refused")),
            ("update: lower-case category", Expect::LogOnly("A-26: the update applies CONFIGURED and logs it (Java logged category=null)")),
            ("batch: ok", Expect::Json(200, json!({"ok":true,"poolIds":[13,14]}), "pool 13 of the recording was refused (A-26)")),
            ("favorite: unknown pool", Expect::Json(400, err("Pool not found: 999"), "A-26 (Java: 500)")),
            ("default: unknown pool", Expect::Json(400, err("Pool not found: 999"), "A-26 (Java: 500)")),
            ("list after edits", Expect::Json(200, pools_after_edits(), "A-25, A-26, N-15, see pools_after_edits")),
            ("remove: ok", Expect::LogOnly("one empty pool fewer to warn about (A-26)")),
            ("remove: last default", Expect::LogOnly("one empty pool fewer to warn about (A-26)")),
            ("list final", Expect::Json(200, list_final, "A-25, A-26, N-15, see pools_after_edits")),
        ],
    )
    .await;
}

#[tokio::test]
async fn submissions_scenario() {
    run(
        "submissions",
        vec![
            (
                "submit: chart not indexed",
                Expect::Json(
                    200,
                    json!({"ok":true,"accepted":2,"results":[
                {"chartId":1002,"accepted":true,"reason":null},
                {"chartId":424242,"accepted":true,"reason":null}]}),
                    "A-15: 424242 is looked up on Phira and indexed (Java refused the whole batch)",
                ),
            ),
            (
                "submit: chart unknown to phira",
                Expect::Json(
                    200,
                    json!({"ok":true,"accepted":0,"results":[
                {"chartId":900005,"accepted":false,"reason":"Phira 上找不到这张谱面"}]}),
                    "A-15: refused per chart",
                ),
            ),
            (
                "mine",
                Expect::Check(
                    |r| {
                        expect_submissions(
                            r,
                            &[
                                (424242, "PENDING", 1),
                                (1002, "PENDING", 2),
                                (1000, "PENDING", 1),
                                (900007, "PENDING", 1),
                                (1001, "PENDING", 2),
                            ],
                        )
                    },
                    "A-15: 1002 and 424242 were accepted",
                ),
            ),
            (
                "mine user 5",
                Expect::Check(
                    |r| expect_submissions(r, &[(1002, "PENDING", 2)]),
                    "A-15: user 2 backs 1002 as well",
                ),
            ),
            (
                "pending",
                Expect::Check(
                    |r| {
                        expect_submissions(
                            r,
                            &[
                                (424242, "PENDING", 1),
                                (1002, "PENDING", 2),
                                (1000, "PENDING", 1),
                                (900007, "PENDING", 1),
                                (1001, "PENDING", 2),
                            ],
                        )
                    },
                    "A-15",
                ),
            ),
            (
                "pool list",
                Expect::Check(
                    |r| {
                        expect_submissions(
                            r,
                            &[
                                (1000, "PENDING", 1),
                                (1001, "PENDING", 2),
                                (1002, "PENDING", 2),
                                (1003, "APPROVED", 1),
                                (424242, "PENDING", 1),
                                (900007, "PENDING", 1),
                            ],
                        )
                    },
                    "A-15",
                ),
            ),
            (
                "pool list pending",
                Expect::Check(
                    |r| {
                        expect_submissions(
                            r,
                            &[
                                (1000, "PENDING", 1),
                                (1001, "PENDING", 2),
                                (1002, "PENDING", 2),
                                (424242, "PENDING", 1),
                                (900007, "PENDING", 1),
                            ],
                        )
                    },
                    "A-15",
                ),
            ),
            (
                "approve: upstream fails",
                Expect::Json(
                    400,
                    err("加入谱池失败：HTTP 404: {\"detail\":\"Not found\"}"),
                    "A-19: the chart could not be added, so the submission was not marked approved",
                ),
            ),
            (
                "pending after",
                Expect::Check(
                    |r| expect_submissions(r, &[(424242, "PENDING", 1), (900007, "PENDING", 1)]),
                    "A-15 (424242 pending) and A-19 (900007 stays pending)",
                ),
            ),
            (
                "pool list after",
                Expect::Check(
                    |r| {
                        expect_submissions(
                            r,
                            &[
                                (1001, "REJECTED", 2),
                                (1002, "APPROVED", 2),
                                (1003, "APPROVED", 1),
                                (424242, "PENDING", 1),
                                (900007, "PENDING", 1),
                            ],
                        )
                    },
                    "A-15, A-19",
                ),
            ),
            (
                "resubmit approved-failed",
                Expect::Json(
                    200,
                    json!({"ok":true,"accepted":0,"results":[
                {"chartId":900007,"accepted":false,"reason":"Phira 上找不到这张谱面"}]}),
                    "A-15: refused per chart",
                ),
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn charts() {
    run(
        "charts",
        vec![
            ("search: tag duplicate", Expect::Json(400, err("参数格式错误：tag"), "A-26 (Java: 500)")),
            ("generate: probe 200", Expect::Json(200, json!({"ok":true,"poolIds":[3],"leftover":0}),
                "A-12: the probe budget is capped at 500, not refused over 20")),
            ("generate: manual probe 200", Expect::Json(400, err("该类别不支持规则生成"),
                "A-12: with no 20-chart refusal, the category check answers")),
            ("generate: size zero", Expect::Json(400, err("Pool size limit must be positive"), "A-26 (Java: 500)")),
            ("generate: regular", Expect::Json(200, json!({"ok":true,"poolIds":[4,5],"leftover":0}),
                "A-24: chart 1000 already sits in REGULAR pool 0; ids follow the TB pool of A-12")),
            ("generate: regular again", Expect::Json(200, json!({"ok":true,"poolIds":[],"leftover":0}),
                "A-24: repeating is additive")),
            ("generate: tb probe 20", Expect::Json(200, json!({"ok":true,"poolIds":[],"leftover":0}),
                "A-24: the TB charts are in pool 3 already")),
            ("generate: lower-case", Expect::Json(200, json!({"ok":true,"poolIds":[6],"leftover":0}),
                "A-26: `configured` is CONFIGURED")),
            ("pools after generate", Expect::Patch(|v| {
                let pools = v["pools"].as_array_mut().unwrap();
                pools.truncate(3);
                for (id, ids, category, size, stay, order) in [
                    (3, json!([1061, 1060, 1003, 1006]), "TB", 15, json!(1), 40),
                    (4, json!([1060, 1004]), "REGULAR", 2, json!(3), 50),
                    (5, json!([1006]), "REGULAR", 2, json!(3), 60),
                    (6, json!([1061, 1003, 1007]), "CONFIGURED", 15, Value::Null, 70),
                ] {
                    pools.push(json!({"id":id,"favoriteId":null,"default":false,"chartIds":ids,
                        "category":category,"sizeLimit":size,"roundsPerStay":stay,"order":order,
                        "submissionOpen":false}));
                }
            }, "A-12, A-24, A-26 as above")),
            ("search: refresh", Expect::Patch(|v| v["result"]["refreshing"] = json!(true),
                "the refresh flag is set before answering; Java's background sweep raced the answer and had already finished")),
        ],
    )
    .await;
}

fn java_dates(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let Ok(t) = JavaOffsetDateTime::parse(s) {
                *s = t.to_string();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(java_dates),
        Value::Object(map) => map.values_mut().for_each(java_dates),
        _ => {}
    }
}

fn sort_records(records: &mut [Value], key: &str, newest_first: bool) {
    records.sort_by_key(|r| r[key].as_i64().unwrap());
    if newest_first {
        records.reverse();
    }
}

#[tokio::test]
async fn data() {
    run(
        "data",
        vec![
            ("export: submissions", Expect::Patch(java_dates,
                "A-03: the export is the store's memory, so dates are written as OffsetDateTime.toString(); Java copied the file, which the seed wrote by hand")),
            ("export after: records", Expect::Patch(|v| sort_records(v.as_array_mut().unwrap(), "finished_at", false),
                "N-02 (ledger): merged rounds are ordered by finish time")),
            ("records after import", Expect::Patch(|v| {
                let records = v["result"]["records"].as_array_mut().unwrap();
                records.insert(0, json!({"id":"r9-1735689900000-9","roomId":"r9","chartId":1004,"chartName":"C1004","startedAt":1735689850000_i64,"finishedAt":1735689900000_i64,"results":[{"playerId":105,"playerName":"Eve","rank":1,"score":1000000,"accuracy":1.0,"std":0.0,"gainedPoints":50,"totalPoints":50}]}));
                records.truncate(2);
            }, "N-02 (ledger): the imported round is the newest")),
            ("import: records wrong kind", Expect::Json(400, err("比赛记录第 1 项无效：必须是对象"),
                "A-01: rows are validated before anything changes (Java: 500, history wiped)")),
            ("records after bad import", Expect::Check(|r| {
                if r.json()["result"]["total"] == json!(4) { Ok(()) } else { Err(r.text()) }
            }, "A-01: the refused import left the history as it was")),
            ("import: submissions null row", Expect::Json(400, err("submissions[0] 无效：不是对象"),
                "A-01 (Java: 200 and every submission gone)")),
            ("import: pools empty", Expect::Json(400, err("pools 至少需要一个谱池"), "A-01 (Java: 500)")),
            ("import: admins wrong rows", Expect::Json(400, err("管理员名单第 1 项无效：必须是整数用户 ID"),
                "A-01 (Java: 200 with an admins.json that no longer loads)")),
            ("restore: kind prefix bypass", Expect::Json(400, err("备份不存在"),
                "O-09: the kind is checked on the file name that is opened (Java restored an admins backup as points)")),
        ],
    )
    .await;
}

#[tokio::test]
async fn admin() {
    run(
        "admin",
        vec![(
            "remove not admin, one left",
            Expect::Json(
                200,
                json!({"ok":true,"removed":false,"admins":[1]}),
                "N-03: removing a non-admin is a no-op, whatever the size of the list",
            ),
        )],
    )
    .await;
}

#[tokio::test]
async fn records() {
    let failed = || {
        Expect::Json(
            400,
            err("登录失败，请检查邮箱与密码"),
            "O-05: a Phira answer without a usable token is a normal login failure (Java: 500)",
        )
    };
    run(
        "records",
        vec![
            ("login: no token from phira", failed()),
            ("login: empty answer from phira", failed()),
            ("login: html answer from phira", failed()),
        ],
    )
    .await;
}

fn chat_enable(room: &mut Value, on: bool) {
    let config = room["config"].as_object_mut().unwrap();
    config.insert("chatEnable".into(), json!(on));
    config.insert("cancelReadyInVote".into(), json!(true));
    config.insert("startWhenAllReady".into(), json!(true));
    config.insert("allReadyDelay".into(), json!(5));
}

fn sort_rooms(v: &mut Value) {
    v["rooms"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|r| r["roomId"].as_str().unwrap().to_owned());
}

#[tokio::test]
async fn rooms() {
    run(
        "rooms",
        vec![
            ("create null pool", Expect::Json(400, err(BAD_FORMAT),
                "A-26: a null pool id is a malformed body (Java: NullPointerException, 500)")),
            ("get", Expect::Patch(|v| chat_enable(&mut v["info"], true), "A-22: config.chatEnable")),
            ("get slash", Expect::Patch(|v| chat_enable(&mut v["info"], true), "A-22")),
            ("list", Expect::Patch(|v| {
                sort_rooms(v);
                for room in v["rooms"].as_array_mut().unwrap() {
                    chat_enable(room, true);
                }
            }, "D2: rooms ascending by id (Java: hash order); A-22")),
            ("get after update", Expect::Patch(|v| {
                v["info"]["locked"] = json!(true);
                chat_enable(&mut v["info"], false);
            }, "N-16: `lock` sets the room's lock (Java ignored it); A-22: chatEnable false")),
            ("get after failed update", Expect::Patch(|v| {
                v["info"]["locked"] = json!(true);
                v["info"]["config"]["minPlayer"] = json!(2);
                v["info"]["config"]["maxPlayer"] = json!(4);
                chat_enable(&mut v["info"], false);
            }, "A-21: the refused update changed nothing (Java had applied minPlayer, maxPlayer and chatEnable before failing); N-16")),
            ("update min above max", Expect::Json(400, err("minPlayer 不能大于 maxPlayer"),
                "A-21 (Java: 200, a room that can never start)")),
            ("get r2", Expect::Patch(|v| {
                v["info"]["config"]["minPlayer"] = json!(2);
                v["info"]["config"]["maxPlayer"] = json!(1000);
                chat_enable(&mut v["info"], true);
            }, "A-21: minPlayer 7 above maxPlayer 3 was refused; A-22")),
            ("clear", Expect::Json(200, json!({"ok":true,"removed":0}),
                "F1: the new room-clear route (Java: unknown route)")),
            ("list final", Expect::Patch(|v| {
                sort_rooms(v);
                for room in v["rooms"].as_array_mut().unwrap() {
                    if room["roomId"] == json!("r2") {
                        room["config"]["minPlayer"] = json!(2);
                        room["config"]["maxPlayer"] = json!(1000);
                    }
                    chat_enable(room, true);
                }
            }, "D2, A-21, A-22 as above")),
        ],
    )
    .await;
}

#[tokio::test]
async fn static_missing() {
    let env = Env::build(Default::default(), |dir| {
        std::fs::remove_dir_all(dir.join("frontend/dist")).unwrap();
    })
    .await;
    run_in(
        env,
        "static_missing",
        vec![(
            "HEAD root",
            Expect::Response(
                |r| head(r, 404, "text/html;charset=utf-8", 9),
                "O-05: HEAD answers what GET answers (Java: 200 without running anything)",
            ),
        )],
    )
    .await;
}
