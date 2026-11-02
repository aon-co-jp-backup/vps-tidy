//! 規則の実行。**消す対象は、固定の規則だけで決める**(AIは使わない)。すべての削除は、同じ安全装置を通る。

use crate::config::{Config, Kind, Rule};
use crate::sys;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum What {
    Deleted,
    WouldDelete,
    Compressed,
    WouldCompress,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Action {
    pub rule: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub what: What,
    pub note: String,
}

pub struct Engine<'a> {
    pub cfg: &'a Config,
    pub apply: bool,
    pub heavy_on: bool,
    open: Vec<PathBuf>,
    build_running: bool,
    unit_text: String,
    pub planned_bytes: u64,
    pub actions: Vec<Action>,
}

fn age_days(meta: &fs::Metadata) -> f64 {
    let m = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    SystemTime::now().duration_since(m).map(|d| d.as_secs_f64() / 86400.0).unwrap_or(0.0)
}

/// フォルダの大きさ(バイト)。シンボリックリンクはたどらない。
pub fn dir_size(p: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(p) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(rd) = fs::read_dir(p) else { return 0 };
    rd.flatten().map(|e| dir_size(&e.path())).sum()
}

fn expand(globs: &[String]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for g in globs {
        if let Ok(paths) = glob::glob(g) {
            v.extend(paths.flatten());
        }
    }
    v.sort();
    v.dedup();
    v
}

impl<'a> Engine<'a> {
    pub fn new(cfg: &'a Config, apply: bool, heavy_on: bool) -> Self {
        Self {
            cfg,
            apply,
            heavy_on,
            open: sys::open_paths(),
            build_running: sys::build_running(),
            unit_text: sys::unit_texts(&cfg.guard.unit_dirs),
            planned_bytes: 0,
            actions: Vec::new(),
        }
    }

    /// テスト用: 開いているファイルなどを、外から与える。
    #[cfg(test)]
    pub fn for_test(cfg: &'a Config, apply: bool, heavy_on: bool) -> Self {
        Self { cfg, apply, heavy_on, open: vec![], build_running: false, unit_text: String::new(), planned_bytes: 0, actions: vec![] }
    }

    fn skip(&mut self, rule: &str, path: &Path, note: impl Into<String>) {
        self.actions.push(Action { rule: rule.to_string(), path: path.to_path_buf(), bytes: 0, what: What::Skipped, note: note.into() });
    }

    /// 触ってはいけないか(`guard.protected` の下、または、それを含むフォルダ)。
    fn protected(&self, path: &Path) -> Option<&PathBuf> {
        let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.cfg.guard.protected.iter().find(|p| {
            let pr = fs::canonicalize(p).unwrap_or_else(|_| (*p).clone());
            real.starts_with(&pr) || pr.starts_with(&real)
        })
    }

    fn in_use(&self, path: &Path) -> bool {
        let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.open.iter().any(|o| o.starts_with(&real))
    }

    fn cap_bytes(&self) -> u64 {
        (self.cfg.guard.max_delete_gb * 1024.0 * 1024.0 * 1024.0) as u64
    }

    /// 共通の安全確認を通ったら、削除する(試し実行なら、予定として記録するだけ)。
    fn remove(&mut self, rule: &str, path: &Path, bytes: u64) {
        if let Some(p) = self.protected(path) {
            let note = format!("保護されたパス({})", p.display());
            self.skip(rule, path, note);
            return;
        }
        if self.in_use(path) {
            self.skip(rule, path, "使用中(プロセスが開いている)");
            return;
        }
        if self.planned_bytes + bytes > self.cap_bytes() {
            self.skip(rule, path, format!("1回の上限({} GB)に達したので、次回に回す", self.cfg.guard.max_delete_gb));
            return;
        }
        self.planned_bytes += bytes;
        if !self.apply {
            self.actions.push(Action { rule: rule.to_string(), path: path.to_path_buf(), bytes, what: What::WouldDelete, note: String::new() });
            return;
        }
        let res = match fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(path),
            Ok(_) => fs::remove_file(path),
            Err(e) => Err(e),
        };
        match res {
            Ok(()) => self.actions.push(Action { rule: rule.to_string(), path: path.to_path_buf(), bytes, what: What::Deleted, note: String::new() }),
            Err(e) => {
                self.planned_bytes -= bytes;
                self.skip(rule, path, format!("削除に失敗: {e}"));
            }
        }
    }

    pub fn run(&mut self) {
        let rules: Vec<Rule> = self.cfg.rules.clone();
        for r in &rules {
            if r.heavy && !self.heavy_on {
                self.skip(&r.name, Path::new("-"), "強めの整理は、ディスク使用率が低いので、今回は動かさない");
                continue;
            }
            match r.kind {
                Kind::OldFiles => self.old_files(r),
                Kind::CompressLogs => self.compress_logs(r),
                Kind::CacheDir => self.cache_dir(r),
                Kind::CargoSlim => self.cargo_slim(r),
            }
        }
    }

    fn old_files(&mut self, r: &Rule) {
        // 同じフォルダ・同じ名前の「控え」を1組として、新しい順に keep_latest 個は残す。
        let mut groups: std::collections::BTreeMap<(PathBuf, String), Vec<(PathBuf, f64, u64)>> = Default::default();
        for p in expand(&r.globs) {
            let Ok(meta) = fs::symlink_metadata(&p) else { continue };
            if !meta.is_file() {
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let stem = ["bak", "before", "old", "orig"].iter().filter_map(|k| name.find(&format!(".{k}"))).min().map(|i| name[..i].to_string()).unwrap_or(name);
            groups.entry((p.parent().map(Path::to_path_buf).unwrap_or_default(), stem)).or_default().push((p, age_days(&meta), meta.len()));
        }
        for (_, mut files) in groups {
            files.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)); // 新しい順(経過日数が小さい順)
            for (i, (p, age, len)) in files.into_iter().enumerate() {
                if i < r.keep_latest {
                    continue;
                }
                if age < r.min_age_days {
                    continue;
                }
                self.remove(&r.name, &p, len);
            }
        }
    }

    fn compress_logs(&mut self, r: &Rule) {
        for p in expand(&r.globs) {
            let Ok(meta) = fs::symlink_metadata(&p) else { continue };
            if !meta.is_file() || p.extension().is_some_and(|e| e == "gz") || age_days(&meta) < r.min_age_days {
                continue;
            }
            if let Some(pp) = self.protected(&p) {
                let note = format!("保護されたパス({})", pp.display());
                self.skip(&r.name, &p, note);
                continue;
            }
            if self.in_use(&p) {
                self.skip(&r.name, &p, "使用中(プロセスが開いている)");
                continue;
            }
            if !self.apply {
                self.actions.push(Action { rule: r.name.clone(), path: p, bytes: meta.len(), what: What::WouldCompress, note: String::new() });
                continue;
            }
            let ok = Command::new("gzip").arg("-f").arg(&p).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                self.actions.push(Action { rule: r.name.clone(), path: p, bytes: meta.len(), what: What::Compressed, note: String::new() });
            } else {
                self.skip(&r.name, &p, "gzip に失敗");
            }
        }
    }

    fn cache_dir(&mut self, r: &Rule) {
        for p in expand(&r.globs) {
            let Ok(meta) = fs::symlink_metadata(&p) else { continue };
            if !meta.is_dir() || age_days(&meta) < r.min_age_days {
                continue;
            }
            let size = dir_size(&p);
            self.remove(&r.name, &p, size);
        }
    }

    fn cargo_slim(&mut self, r: &Rule) {
        for t in expand(&r.globs) {
            if !t.is_dir() {
                continue;
            }
            // 1) ビルドの残りかすと確認できる(cargo が付ける目印がある)
            if !t.join("CACHEDIR.TAG").is_file() {
                self.skip(&r.name, &t, "CACHEDIR.TAG が無い(cargo の target と確認できない)");
                continue;
            }
            // 2) 最終バイナリが release の直下に残る
            let release = t.join("release");
            if !sys::has_executable_file(&release) {
                self.skip(&r.name, &t, "release の直下に実行ファイルが無い(消すと何も残らない)");
                continue;
            }
            // 3) ビルド中ではない
            if self.build_running {
                self.skip(&r.name, &t, "cargo/rustc が動いている(ビルド中かもしれない)");
                continue;
            }
            // 4) 最近、更新されていない
            let fresh = [&t, &release].iter().filter_map(|d| fs::metadata(d).ok()).any(|m| age_days(&m) * 24.0 < r.untouched_hours);
            if fresh {
                self.skip(&r.name, &t, format!("{} 時間以内に更新されている", r.untouched_hours));
                continue;
            }
            // 5) deps / debug の中から動いているサービスが無い(サービス定義に書かれていない)
            let real = fs::canonicalize(&t).unwrap_or_else(|_| t.clone());
            let deps_in_unit = [real.join("release/deps"), real.join("debug")].iter().any(|d| self.unit_text.contains(&*d.to_string_lossy()));
            if deps_in_unit {
                self.skip(&r.name, &t, "サービス定義が deps/debug の中を指している");
                continue;
            }
            // 消す対象: 中間生成物だけ。最終バイナリ(release の直下)には、触らない。
            for sub in ["release/deps", "release/build", "release/.fingerprint", "release/incremental", "release/examples", "debug"] {
                let p = t.join(sub);
                if p.exists() {
                    let size = dir_size(&p);
                    self.remove(&r.name, &p, size);
                }
            }
        }
    }

    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for a in &self.actions {
            match a.what {
                What::Deleted => {
                    s.deleted += 1;
                    s.deleted_bytes += a.bytes;
                }
                What::WouldDelete => {
                    s.would += 1;
                    s.would_bytes += a.bytes;
                }
                What::Compressed => s.compressed += 1,
                What::WouldCompress => s.would_compress += 1,
                What::Skipped => s.skipped += 1,
            }
        }
        s
    }
}

#[derive(Debug, Default)]
pub struct Summary {
    pub deleted: usize,
    pub deleted_bytes: u64,
    pub would: usize,
    pub would_bytes: u64,
    pub compressed: usize,
    pub would_compress: usize,
    pub skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Guard, Kind, Rule};
    use std::os::unix::fs::PermissionsExt;

    fn write(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![b'x'; bytes]).unwrap();
    }

    fn set_old(p: &Path, days: u64) {
        let t = SystemTime::now() - std::time::Duration::from_secs(days * 86400);
        let f = fs::OpenOptions::new().write(true).open(p).or_else(|_| fs::File::open(p)).unwrap();
        f.set_modified(t).unwrap();
    }

    fn rule(name: &str, kind: Kind, glob: &str) -> Rule {
        Rule { name: name.into(), kind, heavy: false, globs: vec![glob.into()], min_age_days: 0.0, keep_latest: 0, untouched_hours: 0.0 }
    }

    #[test]
    fn dry_run_deletes_nothing_and_apply_does() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tmp");
        write(&f, 10);
        let cfg = Config { rules: vec![rule("tmp", Kind::OldFiles, &format!("{}/*.tmp", d.path().display()))], ..Default::default() };
        let mut e = Engine::for_test(&cfg, false, false);
        e.run();
        assert!(f.exists(), "試し実行では消さない");
        assert_eq!(e.summary().would, 1);
        let mut e = Engine::for_test(&cfg, true, false);
        e.run();
        assert!(!f.exists());
        assert_eq!(e.summary().deleted, 1);
    }

    #[test]
    fn keeps_the_latest_backups_and_only_deletes_old_ones() {
        let d = tempfile::tempdir().unwrap();
        for (n, age) in [("app.before-1", 1u64), ("app.before-2", 20), ("app.before-3", 40), ("app.before-4", 60)] {
            let p = d.path().join(n);
            write(&p, 5);
            set_old(&p, age);
        }
        let mut r = rule("bak", Kind::OldFiles, &format!("{}/app.before-*", d.path().display()));
        r.keep_latest = 2;
        r.min_age_days = 30.0;
        let cfg = Config { rules: vec![r], ..Default::default() };
        let mut e = Engine::for_test(&cfg, true, false);
        e.run();
        assert!(d.path().join("app.before-1").exists(), "最新は残す");
        assert!(d.path().join("app.before-2").exists(), "2番目も残す(keep_latest=2)");
        assert!(!d.path().join("app.before-3").exists());
        assert!(!d.path().join("app.before-4").exists());
    }

    #[test]
    fn protected_paths_are_never_touched() {
        let d = tempfile::tempdir().unwrap();
        let keep = d.path().join("data/important.tmp");
        write(&keep, 10);
        let cfg = Config {
            guard: Guard { protected: vec![d.path().join("data")], ..Default::default() },
            rules: vec![rule("tmp", Kind::OldFiles, &format!("{}/**/*.tmp", d.path().display()))],
            ..Default::default()
        };
        let mut e = Engine::for_test(&cfg, true, false);
        e.run();
        assert!(keep.exists());
        assert_eq!(e.summary().skipped, 1);
    }

    #[test]
    fn a_folder_that_contains_a_protected_path_is_not_deleted() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("cache/inner/secret.db"), 10);
        let cfg = Config {
            guard: Guard { protected: vec![d.path().join("cache/inner")], ..Default::default() },
            rules: vec![rule("c", Kind::CacheDir, &format!("{}/cache", d.path().display()))],
            ..Default::default()
        };
        let mut e = Engine::for_test(&cfg, true, false);
        e.run();
        assert!(d.path().join("cache/inner/secret.db").exists(), "保護されたものを含むフォルダは、丸ごと消さない");
    }

    #[test]
    fn the_per_run_cap_stops_further_deletion() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..3 {
            write(&d.path().join(format!("f{i}.tmp")), 600_000);
        }
        let mut cfg = Config { rules: vec![rule("tmp", Kind::OldFiles, &format!("{}/*.tmp", d.path().display()))], ..Default::default() };
        cfg.guard.max_delete_gb = 1_000_000.0 / (1024.0 * 1024.0 * 1024.0); // 約1MB
        let mut e = Engine::for_test(&cfg, true, false);
        e.run();
        let s = e.summary();
        assert_eq!(s.deleted, 1, "600KB×1 だけが上限に収まる");
        assert_eq!(s.skipped, 2);
    }

    fn fake_target(root: &Path) -> PathBuf {
        let t = root.join("svc/target");
        write(&t.join("CACHEDIR.TAG"), 1);
        write(&t.join("release/svc-bin"), 100);
        fs::set_permissions(t.join("release/svc-bin"), fs::Permissions::from_mode(0o755)).unwrap();
        write(&t.join("release/deps/libfoo.rlib"), 5000);
        write(&t.join("release/build/x/out"), 500);
        write(&t.join("release/incremental/y"), 700);
        write(&t.join("debug/z"), 900);
        t
    }

    #[test]
    fn cargo_slim_removes_intermediates_but_keeps_the_final_binary() {
        let d = tempfile::tempdir().unwrap();
        let t = fake_target(d.path());
        let cfg = Config { rules: vec![rule("slim", Kind::CargoSlim, &format!("{}/*/target", d.path().display()))], ..Default::default() };
        let mut e = Engine::for_test(&cfg, true, true);
        e.run();
        assert!(t.join("release/svc-bin").exists(), "最終バイナリは残す");
        assert!(t.join("CACHEDIR.TAG").exists());
        assert!(!t.join("release/deps").exists());
        assert!(!t.join("release/build").exists());
        assert!(!t.join("release/incremental").exists());
        assert!(!t.join("debug").exists());
    }

    #[test]
    fn cargo_slim_refuses_without_the_cachedir_tag_or_a_binary() {
        let d = tempfile::tempdir().unwrap();
        let t = fake_target(d.path());
        fs::remove_file(t.join("CACHEDIR.TAG")).unwrap();
        let cfg = Config { rules: vec![rule("slim", Kind::CargoSlim, &format!("{}/*/target", d.path().display()))], ..Default::default() };
        let mut e = Engine::for_test(&cfg, true, true);
        e.run();
        assert!(t.join("release/deps").exists(), "目印が無ければ触らない");

        write(&t.join("CACHEDIR.TAG"), 1);
        fs::remove_file(t.join("release/svc-bin")).unwrap();
        let mut e = Engine::for_test(&cfg, true, true);
        e.run();
        assert!(t.join("release/deps").exists(), "最終バイナリが無ければ(消すと何も残らないので)触らない");
    }

    #[test]
    fn cargo_slim_refuses_while_a_build_runs_or_a_unit_points_into_deps() {
        let d = tempfile::tempdir().unwrap();
        let t = fake_target(d.path());
        let cfg = Config { rules: vec![rule("slim", Kind::CargoSlim, &format!("{}/*/target", d.path().display()))], ..Default::default() };
        let mut e = Engine::for_test(&cfg, true, true);
        e.build_running = true;
        e.run();
        assert!(t.join("release/deps").exists(), "ビルド中は触らない");

        let mut e = Engine::for_test(&cfg, true, true);
        e.unit_text = format!("ExecStart={}/release/deps/svc-123", fs::canonicalize(&t).unwrap().display());
        e.run();
        assert!(t.join("release/deps").exists(), "サービスが deps から動くなら触らない");
    }

    #[test]
    fn heavy_rules_wait_for_a_high_disk_usage() {
        let d = tempfile::tempdir().unwrap();
        let t = fake_target(d.path());
        let mut r = rule("slim", Kind::CargoSlim, &format!("{}/*/target", d.path().display()));
        r.heavy = true;
        let cfg = Config { rules: vec![r], ..Default::default() };
        let mut e = Engine::for_test(&cfg, true, false);
        e.run();
        assert!(t.join("release/deps").exists(), "使用率が低いときは、強めの整理を動かさない");
        let mut e = Engine::for_test(&cfg, true, true);
        e.run();
        assert!(!t.join("release/deps").exists());
    }

    #[test]
    fn files_in_use_are_not_deleted() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("busy.tmp");
        write(&f, 10);
        let cfg = Config { rules: vec![rule("tmp", Kind::OldFiles, &format!("{}/*.tmp", d.path().display()))], ..Default::default() };
        let mut e = Engine::for_test(&cfg, true, false);
        e.open = vec![fs::canonicalize(&f).unwrap()];
        e.run();
        assert!(f.exists());
    }
}
