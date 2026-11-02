//! 報告(人が読む文章)。規則の結果から、決まった形で組み立てる。

use crate::engine::{Action, Summary, What};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

fn mb(b: u64) -> f64 {
    b as f64 / 1_048_576.0
}

fn gb(b: u64) -> f64 {
    b as f64 / 1_073_741_824.0
}

/// 日本時間(+9時間)の日付と時刻。外のコマンドに頼らない。
pub fn jst_now() -> (String, String) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64 + 9 * 3600;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60))
}

pub struct Inputs<'a> {
    pub apply: bool,
    pub heavy_on: bool,
    pub trigger: f64,
    pub used_before: Option<f64>,
    pub used_after: Option<f64>,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    pub actions: &'a [Action],
    pub summary: &'a Summary,
    pub cap_gb: f64,
}

pub fn render(i: &Inputs) -> String {
    let (date, time) = jst_now();
    let mut s = String::new();
    let mode = if i.apply { "実行(削除する)" } else { "試し実行(何も消していません)" };
    let _ = writeln!(s, "vps-tidy 報告  {date} {time} (日本時間)");
    let _ = writeln!(s, "モード: {mode}");
    if let Some(u) = i.used_before {
        let heavy = if i.heavy_on { "動かした" } else { "動かしていない" };
        let _ = writeln!(s, "ディスク使用率: {u:.0}%(強めの整理の基準 {:.0}% 以上 → {heavy})", i.trigger);
    }
    let _ = writeln!(s, "1回の削除の上限: {:.0} GB", i.cap_gb);
    let _ = writeln!(s);
    for a in i.actions.iter().filter(|a| a.what != What::Skipped) {
        let label = match a.what {
            What::Deleted => "削除",
            What::WouldDelete => "削除予定",
            What::Compressed => "圧縮",
            What::WouldCompress => "圧縮予定",
            What::Skipped => "",
        };
        let _ = writeln!(s, "[{label}] {:>8.1} MB  {}  ({})", mb(a.bytes), a.path.display(), a.rule);
    }
    let skipped: Vec<&Action> = i.actions.iter().filter(|a| a.what == What::Skipped).collect();
    if !skipped.is_empty() {
        let _ = writeln!(s, "\n触らなかったもの:");
        for a in skipped {
            let _ = writeln!(s, "  - {}  ({}) … {}", a.path.display(), a.rule, a.note);
        }
    }
    let sm = i.summary;
    let _ = writeln!(s);
    if i.apply {
        let _ = writeln!(s, "合計: 削除 {} 個 / {:.2} GB、圧縮 {} 個、触らなかった {} 個", sm.deleted, gb(sm.deleted_bytes), sm.compressed, sm.skipped);
        if let (Some(b), Some(a)) = (i.free_before, i.free_after) {
            let _ = writeln!(s, "空き: {:.1} GB → {:.1} GB", gb(b), gb(a));
        }
        if let Some(a) = i.used_after {
            let _ = writeln!(s, "使用率(整理後): {a:.0}%");
        }
    } else {
        let _ = writeln!(s, "合計(予定): 削除 {} 個 / {:.2} GB、圧縮 {} 個、触らなかった {} 個", sm.would, gb(sm.would_bytes), sm.would_compress, sm.skipped);
        let _ = writeln!(s, "実際に消すには `--apply` を付けて実行してください。");
    }
    s
}

/// 報告を書き出して、古い報告を消す。
pub fn write(dir: &Path, text: &str, keep_days: u64) -> std::io::Result<std::path::PathBuf> {
    fs::create_dir_all(dir)?;
    let (date, time) = jst_now();
    let path = dir.join(format!("{date}_{}.txt", time.replace(':', "")));
    fs::write(&path, text)?;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|m| std::time::SystemTime::now().duration_since(m).ok()).is_some_and(|d| d.as_secs() > keep_days * 86400);
            if old && p.extension().is_some_and(|x| x == "txt") {
                let _ = fs::remove_file(p);
            }
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jst_date_has_the_expected_shape() {
        let (d, t) = jst_now();
        assert_eq!(d.len(), 10);
        assert_eq!(&d[4..5], "-");
        assert_eq!(t.len(), 8);
    }
}
