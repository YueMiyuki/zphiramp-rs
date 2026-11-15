mod common;

use common::*;
use zphira_protocol::*;

fn run(kind: &str, input: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let mut buf = input;
    let mut out = Vec::new();
    match kind {
        "sb" => ServerBoundPacket::decode(&mut buf)?.encode(&mut out),
        "cb" => ClientBoundPacket::decode(&mut buf)?.encode(&mut out),
        "state" => GameState::decode(&mut buf)?.encode(&mut out),
        "message" => Message::decode(&mut buf)?.encode(&mut out),
        "judgement" => Judgement::decode(&mut buf)?.encode(&mut out),
        "roominfo" => RoomInfo::decode(&mut buf)?.encode(&mut out),
        "frame" => TouchFrame::decode(&mut buf)?.encode(&mut out),
        other => panic!("unknown kind {other}"),
    }
    Ok(out)
}

#[test]
fn crafted_inputs() {
    let rows = rows("decode.tsv");
    for row in &rows {
        let (kind, name) = (&row[0], &row[1]);
        let input = unhex(&row[2]);
        let what = format!(
            "{kind}.{name} [{}]",
            row[2].chars().take(60).collect::<String>()
        );
        match row[3].as_str() {
            "ok" => {
                let out = run(kind, &input).unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(hex(&out), row[4], "{what}");
            }
            "err" => {
                let err = run(kind, &input).expect_err(&what);
                // Java ran out of memory on a huge declared count (heap dependent); Rust fails on the data.
                if row[4] == "java.lang.OutOfMemoryError" {
                    assert_eq!(
                        err.pipeline_action(),
                        PipelineAction::ExceptionCaught,
                        "{what}: {err}"
                    );
                    continue;
                }
                assert_eq!(
                    err.java_exception(),
                    Some(row[4].as_str()),
                    "{what}: exception class ({err})"
                );
                if row[5] != "-" {
                    assert_eq!(err.to_string(), row[5], "{what}: message");
                }
            }
            other => panic!("bad outcome column {other}"),
        }
    }
    eprintln!("{} crafted inputs", rows.len());
}

fn fuzz(
    name: &str,
    seed: u64,
    bases: &[Vec<u8>],
    decode: impl Fn(&[u8]) -> Result<Vec<u8>, ProtocolError>,
) {
    let expected = lines(name);
    assert_eq!(expected.len(), 6000, "{name}");
    let mut r = Rng(seed);
    let mut kinds = std::collections::BTreeMap::new();
    for (i, want) in expected.iter().enumerate() {
        let base = &bases[r.below(bases.len())];
        let ops = 1 + r.below(3);
        let input = mutate(&mut r, base, ops);
        let got = match decode(&input) {
            Ok(rt) => format!("K:{}", crc32(&rt)),
            Err(e) => code(&e).to_owned(),
        };
        // Java ran out of memory on a huge declared count (heap dependent); Rust fails on the data.
        if want == "M" {
            assert!(
                got != "K" && !got.starts_with("K:"),
                "{name} case {i}: Java OOM but Rust decoded {}",
                hex(&input)
            );
            continue;
        }
        assert_eq!(&got, want, "{name} case {i}: input {}", hex(&input));
        *kinds.entry(got.chars().next().unwrap()).or_insert(0) += 1;
    }
    eprintln!("{name}: {kinds:?}");
}

fn bases(direction: &str) -> Vec<Vec<u8>> {
    rows("packets.tsv")
        .into_iter()
        .filter(|r| r[0].starts_with(direction))
        .map(|r| unhex(&r[1]))
        .collect()
}

#[test]
fn fuzz_serverbound() {
    fuzz("fuzz_sb.txt", 0x5B01, &bases("sb."), |b| {
        let mut buf = b;
        Ok(ServerBoundPacket::decode(&mut buf)?.to_bytes().to_vec())
    });
}

#[test]
fn fuzz_clientbound() {
    fuzz("fuzz_cb.txt", 0xCB01, &bases("cb."), |b| {
        let mut buf = b;
        Ok(ClientBoundPacket::decode(&mut buf)?.to_bytes().to_vec())
    });
}
