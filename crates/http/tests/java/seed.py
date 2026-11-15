# Writes tests/fixtures/seed: the data/ and frontend/ trees both servers start from.
# Run from crates/http: python3 -I tests/java/seed.py
import json, os, pathlib

root = pathlib.Path("tests/fixtures/seed")
data = root / "data"
front = root / "frontend"
data.mkdir(parents=True, exist_ok=True)

def pretty(value):
    # Gson pretty printing: two-space indent, ": ", no HTML escaping issues in this data
    return json.dumps(value, indent=2, ensure_ascii=False)

RATING = {1000: "0.95", 1001: "0.93", 1002: "0.9", 1003: "0.88", 1004: "0.86", 1005: "0.97",
          1006: "0.85", 1007: "0.84", 1008: "0.99", 1009: "0.7", 1060: "0.91", 1061: "0.92", 1062: "0.83"}
DURATION = {1003: 400, 1004: 200, 1006: 361}

def chart(cid):
    c = {"id": cid,
         "name": "Spe<c>ial & 'q' = \"x\" 测试曲" if cid == 1000 else f"C{cid}",
         "level": "AT Lv.17",
         "difficulty": 16.0 if cid % 3 == 0 else 17.5,
         "charter": f"ch{cid}"}
    if cid % 2 == 0:
        c["composer"] = f"comp{cid}"
    if cid == 1001:
        c["description"] = "line1\nline2"
    c.update({"ranked": cid % 2 == 0, "reviewed": True, "stable": True, "stableRequest": False,
              "illustration": f"https://img.example/{cid}.png",
              "file": f"https://files.example/{cid}",
              "uploader": cid % 7})
    if cid != 1009:
        c["tags"] = ["regular"] if cid % 2 == 0 else ["plain"]
    c["rating"] = "@" + RATING[cid] + "@"
    c["ratingCount"] = 3 if cid == 1005 else 10
    c["created"] = "2025-01-02T03:04:05.123456+08:00"
    c["updated"] = "2025-03-04T05:06:07Z"
    c["chartUpdated"] = "2025-03-04T05:06:00Z"
    if cid in DURATION:
        c["duration_seconds"] = DURATION[cid]
    return c

def write(name, text):
    (data / name).write_text(text, encoding="utf-8")

index = {str(cid): chart(cid) for cid in sorted(RATING)}
text = pretty(index)
for cid, r in RATING.items():
    text = text.replace(f'"@{r}@"', r)
write("chart-index.json", text)

write("admins.json", pretty([1]))
write("jwt-secret", "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
write(".duration-probe-v2", "")

write("chart-pools.json", pretty({"pools": [
    {"id": 0, "default_flag": True, "chart_ids": [1000, 1002], "category": "REGULAR",
     "size_limit": 15, "rounds_per_stay": 3, "order": 10, "submission_open": False},
    {"id": 1, "favorite_id": 42, "default_flag": False, "chart_ids": [1001], "category": "CONFIGURED",
     "order": 20, "submission_open": False},
    {"id": 2, "default_flag": False, "chart_ids": [1003], "category": "MANUAL",
     "order": 30, "submission_open": True},
]}))

write("submissions.json", pretty({"submissions": [
    {"pool_id": 1, "chart_id": 1005, "status": "REJECTED",
     "submitters": [{"user_id": 3, "name": "player3", "at": "2025-05-01T10:00:00+08:00"}],
     "created_at": "2025-05-01T10:00:00+08:00", "reviewed_at": "2025-05-03T09:30:00Z",
     "reviewer_id": 1, "reason": "too easy"},
    {"pool_id": 2, "chart_id": 1001, "status": "PENDING",
     "submitters": [{"user_id": 3, "name": "player3", "at": "2025-05-01T10:00:00+08:00"}],
     "created_at": "2025-05-01T10:00:00+08:00"},
    {"pool_id": 2, "chart_id": 1003, "status": "APPROVED",
     "submitters": [{"user_id": 4, "name": "player4", "at": "2025-05-01T11:00:00.5+08:00"}],
     "created_at": "2025-05-01T11:00:00.5+08:00", "reviewed_at": "2025-05-02T10:00:00Z", "reviewer_id": 1},
    {"pool_id": 2, "chart_id": 900007, "status": "PENDING",
     "submitters": [{"user_id": 2, "name": "player2", "at": "2025-05-04T08:00:00Z"}],
     "created_at": "2025-05-04T08:00:00Z"},
]}))

write("player-points.json", pretty({
    "101": {"name": "Alice", "points": 120},
    "102": {"name": "Bob<&>", "points": 120},
    "103": {"name": "Carol", "points": 7},
    "104": {"points": 0},
}))

def result(pid, name, rank, score, acc, std, gained, total):
    return {"player_id": pid, "player_name": name, "rank": rank, "score": score,
            "accuracy": "@" + acc + "@", "std": "@" + std + "@", "gained_points": gained, "total_points": total}

records = [
    {"id": "r1-1735689600000-1", "room_id": "r1", "chart_id": 1000, "chart_name": "C1000",
     "started_at": 1735689500000, "finished_at": 1735689600000,
     "results": [result(101, "Alice", 1, 998765, "0.9876543", "0.012", 100, 100),
                 result(102, "Bob<&>", 2, 900000, "0.95", "0.0", 20, 20)]},
    {"id": "r2-1735689700000-2", "room_id": "r2", "chart_id": 1002, "started_at": 1735689650000,
     "finished_at": 1735689700000,
     "results": [result(103, "Carol", 1, 500000, "0.5", "1.0E-4", 7, 7)]},
    {"id": "r1-1735689800000-3", "room_id": "r1", "chart_id": 1003, "chart_name": "C1003",
     "started_at": 1735689750000, "finished_at": 1735689800000,
     "results": [result(102, "Bob<&>", 1, 999999, "1.0", "0.005", 100, 120),
                 result(101, "Alice", 2, 800000, "0.8999999761581421", "0.25", 20, 120)]},
]
text = pretty(records)
import re
text = re.sub(r'"@([^@"]*)@"', r"\1", text)
write("round-records.json", text)

def put(rel, content):
    p = front / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(content, bytes):
        p.write_bytes(content)
    else:
        p.write_text(content, encoding="utf-8")

put("secret.txt", "outside dist\n")
put("dist/index.html", "<!doctype html><title>root</title>\n")
put("dist/room/__fallback__/index.html", "<!doctype html><title>room</title>\n")
put("dist/pool/__fallback__/index.html", "<!doctype html><title>pool</title>\n")
put("dist/_next/static/chunks/app.js", "".join(f"console.log({i});\n" for i in range(400)))
put("dist/style.css", "body{color:red}\n")
put("dist/logo.jpg", bytes([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0xFF, 0xD9]))
put("dist/data.json", '{"a":1}\n')
put("dist/manifest.webmanifest", '{"name":"zenith"}\n')
put("dist/LICENSE", "license text\n")
put("dist/file.PNG", "not really a png\n")
put("dist/a b.txt", "space\n")
put("dist/中文.txt", "chinese\n")
put("dist/docs/readme.txt", "docs\n")
put("dist/index.txt", "rsc payload\n")
