//! OS の情報(読み取りだけ): 開いているファイル、ビルド中のプロセス、ディスク使用率、サービス定義の文章。
//! Linux と Windows で、取り方が違う部分は、`cfg` で分ける。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------- 開いているファイル

/// すべてのプロセスが、いま開いているファイル(と、実行中の実行ファイル)の実体パス。
/// Windows では取れない(空)。代わりに、消す前に「名前を変えられるか」で使用中を確かめる(engine 側)。
#[cfg(unix)]
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

#[cfg(windows)]
pub fn open_paths() -> Vec<PathBuf> {
    Vec::new()
}

// ---------------------------------------------------------------- ビルド中か

/// `cargo` / `rustc` が動いているか(ビルド中の `target` を触らないため)。
#[cfg(unix)]
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

#[cfg(windows)]
pub fn build_running() -> bool {
    let Ok(out) = Command::new("tasklist").args(["/FO", "CSV", "/NH"]).output() else { return false };
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    ["\"cargo.exe\"", "\"rustc.exe\"", "\"link.exe\""].iter().any(|n| text.contains(n))
}

// ---------------------------------------------------------------- ディスク

/// (使用率%, 空きバイト)。取れなければ None。
#[cfg(unix)]
pub fn disk(mount: &str) -> Option<(f64, u64)> {
    let out = Command::new("df").args(["-P", "-k", mount]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    let used: f64 = cols.get(4)?.trim_end_matches('%').parse().ok()?;
    let free_kb: u64 = cols.get(3)?.parse().ok()?;
    Some((used, free_kb * 1024))
}

/// Windows: ドライブ(例 `F:`)の使用率と空きを、PowerShell の `Get-PSDrive` で取る。
#[cfg(windows)]
pub fn disk(mount: &str) -> Option<(f64, u64)> {
    let letter = mount.trim().trim_end_matches(['\\', '/']).trim_end_matches(':');
    if letter.len() != 1 || !letter.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let script = format!("$d=Get-PSDrive -Name {letter}; \"$($d.Used) $($d.Free)\"");
    let out = Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace();
    let used: f64 = it.next()?.parse().ok()?;
    let free: f64 = it.next()?.parse().ok()?;
    let total = used + free;
    if total <= 0.0 {
        return None;
    }
    Some((used / total * 100.0, free as u64))
}

pub fn used_percent(mount: &str) -> Option<f64> {
    disk(mount).map(|d| d.0)
}

pub fn free_bytes(mount: &str) -> Option<u64> {
    disk(mount).map(|d| d.1)
}

// ---------------------------------------------------------------- サービス定義

/// サービス定義(`*.service`)の文章を、まとめて返す(Windows では、空)。
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

// ---------------------------------------------------------------- 実行ファイル

/// フォルダの直下に、実行できる通常ファイルが1つでもあるか(最終バイナリが残るかの確認)。
#[cfg(unix)]
pub fn has_executable_file(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rd) = fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| e.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

#[cfg(windows)]
pub fn has_executable_file(dir: &Path) -> bool {
    let Ok(rd) = fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) && e.metadata().is_ok_and(|m| m.is_file()))
}
