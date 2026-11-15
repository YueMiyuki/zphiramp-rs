mod support;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use support::server::{Reply, Request, Server};
use support::{cbr128, expected_probe, payload_bytes, read_tsv};
use zphira_phira::duration::{
    AUDIO_FULL_BYTES, AUDIO_PROBE_BYTES, TAIL_BYTES, find_audio_entry, find_info_file,
    info_music_range, list_entries,
};
use zphira_phira::{Options, PhiraClient};

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("test step exceeded its 60 s bound")
}

fn options() -> Options {
    Options {
        probe_timeout: Duration::from_secs(10),
        retry_delay: Duration::from_millis(10),
        ..Options::default()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Cdn {
    Ranges,
    IgnoresRange,
    NoContentRange,
}

struct World {
    api: Server,
    cdn: Server,
    client: PhiraClient,
    ids: HashMap<String, i32>,
}

async fn world(files: Vec<(String, Option<Vec<u8>>)>, cdn_mode: Cdn) -> World {
    let ids: HashMap<String, i32> = files
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (n.clone(), i as i32 + 1))
        .collect();
    let store: Arc<HashMap<String, Vec<u8>>> = Arc::new(
        files
            .iter()
            .filter_map(|(n, d)| d.clone().map(|d| (n.clone(), d)))
            .collect(),
    );
    let cdn = Server::start(move |request, _| {
        let name = request
            .target
            .strip_prefix("/cdn/")
            .and_then(|t| t.split('?').next())
            .unwrap_or("");
        match store.get(name) {
            None => Reply::status(404, "no such file"),
            Some(file) => match cdn_mode {
                Cdn::Ranges => Reply::ranged(request, file),
                Cdn::IgnoresRange => Reply::ok(file.clone()),
                Cdn::NoContentRange => match Reply::ranged(request, file) {
                    Reply::Answer { status, body, .. } => Reply::Answer {
                        status,
                        headers: Vec::new(),
                        body,
                    },
                    other => other,
                },
            },
        }
    })
    .await;
    let cdn_base = cdn.base_url();
    let own = Arc::new(OnceLock::<String>::new());
    let own_for_handler = own.clone();
    let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
    let api = Server::start(move |request, _| {
        let base = own_for_handler.get().expect("own address").clone();
        if let Some(id) = request
            .target
            .strip_prefix("/chart/")
            .and_then(|n| n.parse::<usize>().ok())
        {
            match names.get(id - 1) {
                Some(name) => Reply::json(&format!(
                    r#"{{"id":{id},"name":"c{id}","file":"{base}files/{name}"}}"#
                )),
                None => Reply::status(404, "no chart"),
            }
        } else if let Some(name) = request.target.strip_prefix("/files/") {
            Reply::redirect(303, &format!("{cdn_base}cdn/{name}?sign=abc"))
        } else {
            Reply::status(404, "no route")
        }
    })
    .await;
    own.set(api.base_url()).unwrap();
    let client = PhiraClient::with_options(api.base_url(), options()).unwrap();
    World {
        api,
        cdn,
        client,
        ids,
    }
}

fn ranges(server: &Server) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|r| r.header("range").unwrap_or("<none>").to_owned())
        .collect()
}

fn api_files(world: &World) -> Vec<Request> {
    world
        .api
        .requests()
        .into_iter()
        .filter(|r| r.target.starts_with("/files/"))
        .collect()
}

fn fixture_files() -> Vec<(String, String, String, Option<Vec<u8>>)> {
    read_tsv("duration_probe.tsv")
        .into_iter()
        .map(|row| {
            let [name, category, payload, java] = <[String; 4]>::try_from(row).expect("4 columns");
            let bytes = payload_bytes(&payload);
            (name, category, java, bytes)
        })
        .collect()
}

fn all_files() -> Vec<(String, Option<Vec<u8>>)> {
    fixture_files()
        .into_iter()
        .map(|(n, _, _, b)| (n, b))
        .collect()
}

async fn check_every_file(cdn: Cdn) {
    let world = world(all_files(), cdn).await;
    for (name, category, java, bytes) in fixture_files() {
        let id = world.ids[&name];
        let result = bounded(world.client.probe_duration(id)).await;
        let expected = expected_probe(&name);
        match (&result, expected) {
            (Ok(seconds), Some(want)) => {
                assert_eq!(*seconds, want, "{name} ({category}, Java: {java})")
            }
            (Err(error), None) => {
                let text = error.to_string();
                if bytes.is_none() {
                    assert_eq!(text, "HTTP 404 for chart archive", "{name}");
                } else {
                    assert_eq!(
                        text,
                        format!("Unable to read audio duration: {id}"),
                        "{name}"
                    );
                }
            }
            (result, expected) => {
                panic!("{name}: expected {expected:?}, got {result:?} (Java: {java})")
            }
        }
        let rust_text = expected.map_or("null".to_owned(), |v| v.to_string());
        if category == "same" && bytes.is_some() {
            assert_eq!(java, rust_text, "{name}");
        } else if category.starts_with("A-") || category.starts_with("N-") {
            assert_ne!(java, rust_text, "{name}: a finding differs from Java");
        }
    }
}

#[tokio::test]
async fn probe_matches_java_through_a_303_to_a_range_server() {
    check_every_file(Cdn::Ranges).await;
}

#[tokio::test]
async fn probe_works_when_the_origin_ignores_range() {
    check_every_file(Cdn::IgnoresRange).await;
}

#[tokio::test]
async fn probe_without_content_range() {
    let world = world(
        vec![
            ("zip".into(), Some(support_file("zip_deflated_cbr.zip"))),
            ("bare_big".into(), Some(cbr128(400 * 16_000))),
            ("bare_small".into(), Some(cbr128(20_000))),
        ],
        Cdn::NoContentRange,
    )
    .await;
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 5);
    assert!(bounded(world.client.probe_duration(2)).await.is_err());
    assert_eq!(bounded(world.client.probe_duration(3)).await.unwrap(), 1);
}

fn support_file(name: &str) -> Vec<u8> {
    support::read_fixture_bytes(name)
}

fn info_range(zip: &[u8]) -> Option<String> {
    let info = find_info_file(&list_entries(zip))?;
    let (start, len) = info_music_range(&info)?;
    assert_eq!(
        len,
        30 + 1024 + info.entry.path.len() + info.entry.compressed_size as usize
    );
    Some(format!("bytes={start}-{}", start + len as u64 - 1))
}

#[tokio::test]
async fn probe_issues_the_expected_range_requests() {
    let zip = support_file("zip_deflated_cbr.zip");
    let offset = find_audio_entry(&zip).unwrap().local_offset;
    let info = info_range(&zip).expect("the archive has an info.yml");
    let world = world(vec![("chart.zip".into(), Some(zip))], Cdn::Ranges).await;
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 5);

    assert_eq!(
        ranges(&world.cdn),
        [
            "bytes=0-32767".to_owned(),
            format!("bytes=-{TAIL_BYTES}"),
            info,
            format!("bytes={offset}-{}", offset + AUDIO_PROBE_BYTES as u64),
        ]
    );
    assert_eq!(TAIL_BYTES, 524_288);
    let files = api_files(&world);
    assert_eq!(files.len(), 4);
    assert_eq!(
        world.api.request_count(),
        5,
        "the chart lookup plus four file requests"
    );
    assert_eq!(world.api.requests()[0].target, "/chart/1");
    for (hop, cdn) in files.iter().zip(world.cdn.requests()) {
        assert_eq!(hop.method, "GET");
        assert_eq!(hop.target, "/files/chart.zip");
        assert_eq!(hop.header("range"), cdn.header("range"));
        assert_eq!(hop.header("user-agent"), Some("JPhira/1"));
        assert_eq!(cdn.method, "GET");
        assert_eq!(cdn.target, "/cdn/chart.zip?sign=abc");
        assert_eq!(cdn.header("user-agent"), Some("JPhira/1"));
        assert_eq!(cdn.header("authorization"), None);
    }
}

#[tokio::test]
async fn probe_widens_to_the_second_window() {
    let zip = support_file("zip_deflated_id3_cover.zip");
    let offset = find_audio_entry(&zip).unwrap().local_offset;
    let info = info_range(&zip).unwrap();
    let world = world(vec![("cover.zip".into(), Some(zip))], Cdn::Ranges).await;
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 4);
    assert_eq!(
        ranges(&world.cdn),
        [
            "bytes=0-32767".to_owned(),
            format!("bytes=-{TAIL_BYTES}"),
            info,
            format!("bytes={offset}-{}", offset + AUDIO_PROBE_BYTES as u64),
            format!("bytes={offset}-{}", offset + AUDIO_FULL_BYTES as u64),
        ]
    );
}

#[tokio::test]
async fn a_bare_file_takes_one_request_and_an_archive_without_audio_two() {
    let world = world(
        vec![
            ("bare.mp3".into(), Some(cbr128(400 * 16_000))),
            ("none.zip".into(), Some(support_file("zip_no_audio.zip"))),
        ],
        Cdn::Ranges,
    )
    .await;
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 400);
    assert_eq!(ranges(&world.cdn), ["bytes=0-32767"]);
    let error = bounded(world.client.probe_duration(2)).await.unwrap_err();
    assert_eq!(error.to_string(), "Unable to read audio duration: 2");
    assert_eq!(
        ranges(&world.cdn)[1..],
        ["bytes=0-32767".to_owned(), format!("bytes=-{TAIL_BYTES}")]
    );
}

#[tokio::test]
async fn durations_are_cached_and_failures_are_not() {
    let world = world(
        vec![
            ("ok.zip".into(), Some(support_file("zip_deflated_cbr.zip"))),
            ("bad.bin".into(), Some(b"this is not audio".to_vec())),
        ],
        Cdn::Ranges,
    )
    .await;
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 5);
    let requests = world.api.request_count() + world.cdn.request_count();
    assert_eq!(requests, 1 + 4 + 4);
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 5);
    assert_eq!(
        world.api.request_count() + world.cdn.request_count(),
        requests
    );

    assert!(bounded(world.client.probe_duration(2)).await.is_err());
    let after_first = world.cdn.request_count();
    assert_eq!(after_first, 4 + 1);
    assert!(bounded(world.client.probe_duration(2)).await.is_err());
    assert_eq!(world.cdn.request_count(), after_first + 1);
    let error = bounded(world.client.probe_duration(99)).await.unwrap_err();
    assert_eq!(error.to_string(), "HTTP 404: no chart");
}

#[tokio::test]
async fn charts_without_an_archive_are_a_failed_probe() {
    for file in ["null", r#""""#, r#""   ""#] {
        let server =
            Server::start(move |_, _| Reply::json(&format!(r#"{{"id":5,"file":{file}}}"#))).await;
        let phira = PhiraClient::with_options(server.base_url(), options()).unwrap();
        let error = bounded(phira.probe_duration(5)).await.unwrap_err();
        assert_eq!(error.to_string(), "Chart has no archive: 5", "{file}");
        assert_eq!(server.request_count(), 1, "only the chart lookup");
    }
    let server = Server::start(|_, _| Reply::json(r#"{"id":5,"file":"not a url"}"#)).await;
    let phira = PhiraClient::with_options(server.base_url(), options()).unwrap();
    assert!(bounded(phira.probe_duration(5)).await.is_err());
}

#[tokio::test]
async fn probe_errors_are_not_retried() {
    for status in [404u16, 416, 500, 503, 403] {
        let cdn = Server::start(move |_, _| Reply::status(status, "x")).await;
        let cdn_base = cdn.base_url();
        let api = Server::start(move |request, _| {
            if request.target.starts_with("/chart/") {
                Reply::json(&format!(r#"{{"id":1,"file":"{cdn_base}cdn/x"}}"#))
            } else {
                Reply::status(404, "")
            }
        })
        .await;
        let phira = PhiraClient::with_options(api.base_url(), options()).unwrap();
        let error = bounded(phira.probe_duration(1)).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("HTTP {status} for chart archive")
        );
        assert_eq!(cdn.request_count(), 1, "status {status}");
    }
}

#[tokio::test]
async fn probe_transport_failures_are_not_retried() {
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let api = Server::start(move |_, _| {
        Reply::json(&format!(
            r#"{{"id":1,"file":"http://127.0.0.1:{dead}/never"}}"#
        ))
    })
    .await;
    let phira = PhiraClient::with_options(api.base_url(), options()).unwrap();
    let started = Instant::now();
    let error = bounded(phira.probe_duration(1)).await.unwrap_err();
    assert!(error.is_transport(), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "no backoff: {:?}",
        started.elapsed()
    );

    let cdn = Server::start(|_, _| Reply::Drop).await;
    let cdn_base = cdn.base_url();
    let api =
        Server::start(move |_, _| Reply::json(&format!(r#"{{"id":1,"file":"{cdn_base}x"}}"#)))
            .await;
    let phira = PhiraClient::with_options(api.base_url(), options()).unwrap();
    let error = bounded(phira.probe_duration(1)).await.unwrap_err();
    assert!(error.is_transport(), "{error}");
    assert_eq!(cdn.request_count(), 1);

    let own = Arc::new(OnceLock::<String>::new());
    let own_for_handler = own.clone();
    let api = Server::start(move |request, _| {
        if request.target.starts_with("/chart/") {
            Reply::json(&format!(
                r#"{{"id":1,"file":"{}files/x"}}"#,
                own_for_handler.get().unwrap()
            ))
        } else {
            Reply::redirect(303, &format!("http://127.0.0.1:{dead}/gone"))
        }
    })
    .await;
    own.set(api.base_url()).unwrap();
    let phira = PhiraClient::with_options(api.base_url(), options()).unwrap();
    let error = bounded(phira.probe_duration(1)).await.unwrap_err();
    assert!(error.is_transport(), "{error}");
    assert_eq!(
        api.request_count(),
        2,
        "the chart lookup and one /files request"
    );
}

#[tokio::test]
async fn probe_requests_time_out() {
    let cdn = Server::start(|_, _| Reply::Hang).await;
    let cdn_base = cdn.base_url();
    let api =
        Server::start(move |_, _| Reply::json(&format!(r#"{{"id":1,"file":"{cdn_base}x"}}"#)))
            .await;
    let phira = PhiraClient::with_options(
        api.base_url(),
        Options {
            probe_timeout: Duration::from_millis(250),
            ..options()
        },
    )
    .unwrap();
    let started = Instant::now();
    let error = bounded(phira.probe_duration(1)).await.unwrap_err();
    assert!(
        error.is_transport() && error.to_string().contains("timed out"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(cdn.request_count(), 1);
}

#[tokio::test]
async fn probe_reads_only_the_window_from_an_oversized_answer() {
    let big = cbr128(48 * 1024 * 1024);
    let world = world(vec![("big.mp3".into(), Some(big))], Cdn::IgnoresRange).await;
    let started = Instant::now();
    assert_eq!(
        bounded(world.client.probe_duration(1)).await.unwrap(),
        48 * 1024 * 1024 / 16_000
    );
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probes_run_concurrently() {
    let files: Vec<(String, Option<Vec<u8>>)> = (0..12)
        .map(|i| {
            (
                format!("f{i}.zip"),
                Some(support_file(if i % 2 == 0 {
                    "zip_deflated_cbr.zip"
                } else {
                    "zip_deflated_cbr_192.zip"
                })),
            )
        })
        .collect();
    let world = world(files, Cdn::Ranges).await;
    let calls: Vec<_> = (1..=12)
        .map(|id| {
            let client = world.client.clone();
            tokio::spawn(async move { client.probe_duration(id).await })
        })
        .collect();
    for (i, call) in calls.into_iter().enumerate() {
        let seconds = bounded(call).await.unwrap().unwrap();
        assert_eq!(seconds, if i % 2 == 0 { 5 } else { 3 }, "chart {}", i + 1);
    }
}

#[tokio::test]
async fn probe_follows_at_most_four_redirects() {
    for (hops, ok) in [(1usize, true), (4, true), (5, false), (6, false)] {
        let own = Arc::new(OnceLock::<String>::new());
        let base = own.clone();
        let server = Server::start(move |request, _| {
            let base = base.get().unwrap();
            if request.target.starts_with("/chart/") {
                Reply::json(&format!(r#"{{"id":1,"file":"{base}hop/{hops}"}}"#))
            } else if let Some(n) = request
                .target
                .strip_prefix("/hop/")
                .and_then(|n| n.parse::<usize>().ok())
            {
                if n == 0 {
                    Reply::ranged(request, &cbr128(20_000))
                } else {
                    Reply::redirect(302, &format!("{base}hop/{}", n - 1))
                }
            } else {
                Reply::status(404, "")
            }
        })
        .await;
        own.set(server.base_url()).unwrap();
        let phira = PhiraClient::with_options(server.base_url(), options()).unwrap();
        let result = bounded(phira.probe_duration(1)).await;
        if ok {
            assert_eq!(result.unwrap(), 1, "{hops} redirects");
            assert_eq!(server.request_count(), 1 + hops + 1);
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                "HTTP 302 for chart archive",
                "{hops} redirects"
            );
            assert_eq!(server.request_count(), 1 + 5, "{hops} redirects");
        }
    }
}

#[tokio::test]
async fn probe_request_sequences_with_an_info_file() {
    fn window(zip: &[u8], name: &str) -> String {
        let entry = list_entries(zip)
            .into_iter()
            .find(|e| e.path == name)
            .unwrap_or_else(|| panic!("no entry {name}"));
        format!(
            "bytes={}-{}",
            entry.local_offset,
            entry.local_offset + AUDIO_PROBE_BYTES as u64
        )
    }
    let head = "bytes=0-32767".to_owned();
    let tail = format!("bytes=-{TAIL_BYTES}");

    let cases: &[(&str, Option<&str>, Option<i32>)] = &[
        ("zip_song_second_yml.zip", Some("song.mp3"), Some(5)),
        ("zip_song_second_txt.zip", Some("song.mp3"), Some(5)),
        ("zip_song_second_both.zip", Some("song.mp3"), Some(5)),
        ("zip_song_folder.zip", Some("chart/song.mp3"), Some(5)),
        ("zip_song_case_insensitive.zip", Some("Song.Mp3"), Some(5)),
        ("zip_song_quoted_bom_crlf.zip", Some("song.mp3"), Some(5)),
        ("zip_song_info_stored.zip", Some("song.mp3"), Some(5)),
        ("zip_song_ogg.zip", Some("song.ogg"), Some(5)),
        (
            "zip_info_names_missing_file.zip",
            Some("preview.mp3"),
            Some(2),
        ),
        ("zip_info_no_music.zip", Some("preview.mp3"), Some(2)),
        ("zip_info_garbage.zip", Some("preview.mp3"), Some(2)),
        ("zip_info_names_wav.zip", None, None),
    ];
    for (fixture, audio, expected) in cases {
        let zip = support_file(fixture);
        let info = info_range(&zip).unwrap_or_else(|| panic!("{fixture}: no readable info file"));
        let world = world(vec![("chart.zip".into(), Some(zip.clone()))], Cdn::Ranges).await;
        let result = bounded(world.client.probe_duration(1)).await;
        let mut sequence = vec![head.clone(), tail.clone(), info];
        if let Some(audio) = audio {
            sequence.push(window(&zip, audio));
        }
        assert_eq!(ranges(&world.cdn), sequence, "{fixture}");
        match expected {
            Some(seconds) => assert_eq!(result.unwrap(), *seconds, "{fixture}"),
            None => assert_eq!(
                result.unwrap_err().to_string(),
                "Unable to read audio duration: 1",
                "{fixture}"
            ),
        }
        assert_eq!(api_files(&world).len(), sequence.len(), "{fixture}");
        for request in world.cdn.requests() {
            let mut names: Vec<&str> = request.headers.iter().map(|(n, _)| n.as_str()).collect();
            names.sort_unstable();
            assert_eq!(names, ["host", "range", "user-agent"], "{fixture}");
        }
    }

    for (fixture, audio, expected) in [
        ("zip_no_info_preview_first.zip", "preview.mp3", 2),
        ("zip_info_too_big.zip", "preview.mp3", 2),
        ("zip_first_audio_wins.zip", "a.ogg", 5),
        ("zip_upper_audio_last.zip", "Music.MP3", 5),
    ] {
        let zip = support_file(fixture);
        assert_eq!(info_range(&zip), None, "{fixture}");
        let world = world(vec![("chart.zip".into(), Some(zip.clone()))], Cdn::Ranges).await;
        assert_eq!(
            bounded(world.client.probe_duration(1)).await.unwrap(),
            expected,
            "{fixture}"
        );
        assert_eq!(
            ranges(&world.cdn),
            [head.clone(), tail.clone(), window(&zip, audio)],
            "{fixture}"
        );
        assert_eq!(api_files(&world).len(), 3, "{fixture}");
    }

    let zip = support_file("zip_no_audio.zip");
    assert!(find_info_file(&list_entries(&zip)).is_some());
    let none = world(vec![("chart.zip".into(), Some(zip))], Cdn::Ranges).await;
    assert!(bounded(none.client.probe_duration(1)).await.is_err());
    assert_eq!(ranges(&none.cdn), [head.clone(), tail.clone()]);

    let bare = world(
        vec![("bare.mp3".into(), Some(cbr128(400 * 16_000)))],
        Cdn::Ranges,
    )
    .await;
    assert_eq!(bounded(bare.client.probe_duration(1)).await.unwrap(), 400);
    assert_eq!(ranges(&bare.cdn), [head]);
}

#[tokio::test]
async fn the_info_request_works_against_an_origin_that_ignores_range() {
    let zip = support_file("zip_song_second_yml.zip");
    let world = world(vec![("chart.zip".into(), Some(zip))], Cdn::IgnoresRange).await;
    assert_eq!(bounded(world.client.probe_duration(1)).await.unwrap(), 5);
    assert_eq!(world.cdn.request_count(), 4);
}

#[tokio::test]
async fn a_failing_info_request_fails_the_probe() {
    let zip = support_file("zip_song_second_yml.zip");
    let info = info_range(&zip).unwrap();
    let store = zip.clone();
    let cdn = Server::start(move |request, _| {
        if request.header("range") == Some(info.as_str()) {
            Reply::status(503, "busy")
        } else {
            Reply::ranged(request, &store)
        }
    })
    .await;
    let cdn_base = cdn.base_url();
    let api =
        Server::start(move |_, _| Reply::json(&format!(r#"{{"id":1,"file":"{cdn_base}x"}}"#)))
            .await;
    let phira = PhiraClient::with_options(api.base_url(), options()).unwrap();
    let error = bounded(phira.probe_duration(1)).await.unwrap_err();
    assert_eq!(error.to_string(), "HTTP 503 for chart archive");
    assert_eq!(
        cdn.request_count(),
        3,
        "head, tail, the failed info request"
    );
}
