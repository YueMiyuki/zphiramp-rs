use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use flate2::Compression;
use flate2::write::GzEncoder;

const LATEST: &str = "latest.log";

pub(crate) struct RollingFile {
    dir: PathBuf,
    writer: Option<BufWriter<File>>,
    period: NaiveDate,
    compressions: Vec<JoinHandle<()>>,
}

impl RollingFile {
    pub(crate) fn open(dir: &Path, started: SystemTime, now: DateTime<Local>) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let mut rf = RollingFile {
            dir: dir.to_path_buf(),
            writer: None,
            period: now.date_naive(),
            compressions: Vec::new(),
        };
        let latest = rf.latest_path();
        if let Ok(meta) = fs::metadata(&latest)
            && meta.is_file()
            && meta.len() >= 1
        {
            let modified = meta.modified()?;
            if modified < started {
                // Archived under the date it was last written, not today.
                let date = DateTime::<Local>::from(modified).date_naive();
                rf.archive(date)?;
            }
        }
        rf.open_latest()?;
        Ok(rf)
    }

    fn latest_path(&self) -> PathBuf {
        self.dir.join(LATEST)
    }

    fn open_latest(&mut self) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.latest_path())?;
        self.writer = Some(BufWriter::with_capacity(256 * 1024, file));
        Ok(())
    }

    pub(crate) fn write(&mut self, date: NaiveDate, text: &str) -> io::Result<()> {
        if date > self.period {
            self.roll_over(date)?;
        }
        let w = match self.writer.as_mut() {
            Some(w) => w,
            None => {
                self.open_latest()?;
                self.writer.as_mut().expect("just opened")
            }
        };
        w.write_all(text.as_bytes())
    }

    fn roll_over(&mut self, new_period: NaiveDate) -> io::Result<()> {
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }
        let old_period = self.period;
        self.period = new_period;
        let latest = self.latest_path();
        if fs::metadata(&latest).map(|m| m.len() > 0).unwrap_or(false) {
            self.archive(old_period)?;
        }
        self.open_latest()
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        match self.writer.as_mut() {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }

    pub(crate) fn close(&mut self) -> io::Result<()> {
        let result = match self.writer.take() {
            Some(mut w) => w.flush(),
            None => Ok(()),
        };
        for h in self.compressions.drain(..) {
            let _ = h.join();
        }
        result
    }

    fn archive(&mut self, date: NaiveDate) -> io::Result<()> {
        let index = next_index(&self.dir, date)?;
        let plain = self.dir.join(archive_name(date, index, false));
        fs::rename(self.latest_path(), &plain)?;
        let gz = self.dir.join(archive_name(date, index, true));
        self.compressions.retain(|h| !h.is_finished());
        self.compressions.push(std::thread::spawn(move || {
            if compress(&plain, &gz).is_ok() {
                let _ = fs::remove_file(&plain);
            } else {
                let _ = fs::remove_file(&gz);
            }
        }));
        Ok(())
    }
}

fn archive_name(date: NaiveDate, index: u32, gz: bool) -> String {
    format!(
        "{}-{index}.log{}",
        date.format("%Y-%m-%d"),
        if gz { ".gz" } else { "" }
    )
}

fn compress(src: &Path, dst: &Path) -> io::Result<()> {
    let mut input = File::open(src)?;
    let out = File::create(dst)?;
    let mut enc = GzEncoder::new(BufWriter::new(out), Compression::default());
    io::copy(&mut input, &mut enc)?;
    let mut out = enc.finish()?;
    out.flush()?;
    out.get_ref().sync_all()
}

fn eligible_files(dir: &Path, date: NaiveDate) -> io::Result<BTreeMap<u32, PathBuf>> {
    let prefix = format!("{}-", date.format("%Y-%m-%d"));
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || !rest[digits.len()..].starts_with(".log") {
            continue;
        }
        if let Ok(index) = digits.parse::<u32>() {
            found.insert(index, entry.path());
        }
    }
    Ok(found)
}

// Archives are never deleted or renumbered.
fn next_index(dir: &Path, date: NaiveDate) -> io::Result<u32> {
    Ok(eligible_files(dir, date)?
        .last_key_value()
        .map_or(1, |(&last, _)| last + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration;

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn gunzip(p: &Path) -> String {
        let mut s = String::new();
        io::Read::read_to_string(
            &mut flate2::read::GzDecoder::new(File::open(p).unwrap()),
            &mut s,
        )
        .unwrap();
        s
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn startup_rolls_old_file_under_its_modification_date() {
        let dir = tempfile::tempdir().unwrap();
        let latest = dir.path().join(LATEST);
        fs::write(&latest, "old content\n").unwrap();
        let mtime = Local.with_ymd_and_hms(2026, 11, 11, 23, 59, 30).unwrap();
        File::options()
            .write(true)
            .open(&latest)
            .unwrap()
            .set_modified(mtime.into())
            .unwrap();
        let now = Local.with_ymd_and_hms(2026, 11, 15, 0, 8, 0).unwrap();
        let mut rf = RollingFile::open(dir.path(), SystemTime::now(), now).unwrap();
        rf.close().unwrap();
        assert_eq!(names(dir.path()), vec!["2026-11-11-1.log.gz", "latest.log"]);
        assert_eq!(
            gunzip(&dir.path().join("2026-11-11-1.log.gz")),
            "old content\n"
        );
    }

    #[test]
    fn startup_keeps_empty_or_fresh_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LATEST), "").unwrap();
        let mut rf = RollingFile::open(
            dir.path(),
            SystemTime::now() + Duration::from_secs(5),
            Local::now(),
        )
        .unwrap();
        rf.close().unwrap();
        assert_eq!(names(dir.path()), vec!["latest.log"]);
        fs::write(dir.path().join(LATEST), "x\n").unwrap();
        let started = SystemTime::now() - Duration::from_secs(60);
        let mut rf = RollingFile::open(dir.path(), started, Local::now()).unwrap();
        rf.write(Local::now().date_naive(), "y\n").unwrap();
        rf.close().unwrap();
        assert_eq!(names(dir.path()), vec!["latest.log"]);
        assert_eq!(
            fs::read_to_string(dir.path().join(LATEST)).unwrap(),
            "x\ny\n"
        );
    }

    #[test]
    fn archives_are_never_deleted_or_renumbered() {
        let dir = tempfile::tempdir().unwrap();
        let d = date(2026, 11, 15);
        let mut contents = Vec::new();
        for run in 1..=9 {
            fs::write(dir.path().join(LATEST), format!("run {run}\n")).unwrap();
            let mut rf = RollingFile {
                dir: dir.path().to_path_buf(),
                writer: None,
                period: d,
                compressions: Vec::new(),
            };
            rf.archive(d).unwrap();
            rf.close().unwrap();
            contents.push(run);
        }
        let mut expected: Vec<String> = (1..=9).map(|i| format!("2026-11-15-{i}.log.gz")).collect();
        expected.sort();
        assert_eq!(names(dir.path()), expected);
        for run in 1..=9 {
            assert_eq!(
                gunzip(&dir.path().join(format!("2026-11-15-{run}.log.gz"))),
                format!("run {run}\n")
            );
        }
    }

    #[test]
    fn gaps_and_uncompressed_files_count() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("2026-11-15-3.log.gz"), "x").unwrap();
        fs::write(dir.path().join("2026-11-15-5.log"), "y").unwrap();
        fs::write(dir.path().join("2026-11-14-9.log.gz"), "z").unwrap();
        fs::write(dir.path().join("2026-11-12-07.log.gz"), "w").unwrap();
        assert_eq!(next_index(dir.path(), date(2026, 11, 15)).unwrap(), 6);
        assert_eq!(next_index(dir.path(), date(2026, 11, 14)).unwrap(), 10);
        assert_eq!(next_index(dir.path(), date(2026, 11, 13)).unwrap(), 1);
        assert_eq!(next_index(dir.path(), date(2026, 11, 12)).unwrap(), 8);
        assert_eq!(
            names(dir.path()),
            vec![
                "2026-11-12-07.log.gz",
                "2026-11-14-9.log.gz",
                "2026-11-15-3.log.gz",
                "2026-11-15-5.log"
            ]
        );
    }

    #[test]
    fn date_change_rolls_under_the_previous_period() {
        let dir = tempfile::tempdir().unwrap();
        let day1 = Local.with_ymd_and_hms(2026, 11, 14, 23, 0, 0).unwrap();
        let mut rf = RollingFile::open(dir.path(), SystemTime::now(), day1).unwrap();
        rf.write(day1.date_naive(), "late\n").unwrap();
        rf.write(date(2026, 11, 15), "after midnight\n").unwrap();
        rf.write(date(2026, 11, 18), "days later\n").unwrap();
        rf.close().unwrap();
        assert_eq!(
            names(dir.path()),
            vec!["2026-11-14-1.log.gz", "2026-11-15-1.log.gz", "latest.log"]
        );
        assert_eq!(gunzip(&dir.path().join("2026-11-14-1.log.gz")), "late\n");
        assert_eq!(
            gunzip(&dir.path().join("2026-11-15-1.log.gz")),
            "after midnight\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(LATEST)).unwrap(),
            "days later\n"
        );
    }
}
