//! OS の情報(読み取りだけ): 開いているファイル、ビルド中のプロセス、ディスク使用率、サービス定義の文章。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// すべてのプロセスが、いま開いているファイル(と、実行中の実行ファイル)の実体パス。
pub fn open_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(procs) = fs::read_dir("/proc") else { return out };
    for p in procs.flatten() {
        let name = p.file_name();
        if !name.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(exe) = fs::read_link(p.path().join("exe")) {
            out.push(exe);
        }
        if let Ok(fds) = fs::read_dir(p.path().join("fd")) {
            for fd in fds.flatten() {
                if let Ok(t) = fs::read_link(fd.path()) {
                    if t.is_absolute() {
                        out.push(t);
                    }
                }
            }
        }
    }
    out
}

/// `cargo` / `rustc` が動いているか(ビルド中の `target` を触らないため)。
pub fn build_running() -> bool {
    let Ok(procs) = fs::read_dir("/proc") else { return false };
    for p in procs.flatten() {
        if let Ok(comm) = fs::read_to_string(p.path().join("comm")) {
            let c = comm.trim();
            if c == "cargo" || c == "rustc" {
                return true;
            }
        }
    }
    false
}

/// `df -P -k <mount>` の使用率(%)。取れなければ None。
pub fn used_percent(mount: &str) -> Option<f64> {
    let out = Command::new("df").args(["-P", "-k", mount]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let col = line.split_whitespace().nth(4)?;
    col.trim_end_matches('%').parse().ok()
}

/// `df -P -k <mount>` の空き(バイト)。
pub fn free_bytes(mount: &str) -> Option<u64> {
    let out = Command::new("df").args(["-P", "-k", mount]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// サービス定義(`*.service`)の文章を、まとめて返す。
pub fn unit_texts(dirs: &[PathBuf]) -> String {
    let mut all = String::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "service") {
                if let Ok(t) = fs::read_to_string(&p) {
                    all.push_str(&t);
                    all.push('\n');
                }
            }
        }
    }
    all
}

/// フォルダの中に、実行できる通常ファイルが1つでもあるか(最終バイナリが残るかの確認)。
pub fn has_executable_file(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rd) = fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        e.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}
