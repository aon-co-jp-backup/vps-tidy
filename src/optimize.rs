//! ディスクの最適化(SSD の TRIM、HDD の断片化の解消)。
//!
//! - Linux: `fstrim`(util-linux)で、マウント中のファイルシステムを TRIM する。HDD の断片化の解消は、
//!   ext4/xfs では通常不要なので、行わない。
//! - Windows: `Optimize-Volume`(SSD は `-ReTrim`、HDD は `-Defrag`)。管理者権限が要る。Windows 標準の
//!   「ドライブの最適化」(`ScheduledDefrag`)が有効かも、あわせて確認する。
//! - macOS: APFS が自動で TRIM するので、何もしない。
//!
//! 既定は**計画だけ**(何も実行しない)。`apply` を付けたときだけ実行する。

use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct OptItem {
    /// 対象(Linux: マウント先 / Windows: ドライブ文字)。
    pub target: String,
    /// 行う操作(`TRIM` / `ReTrim` / `Defrag`)。
    pub action: String,
    /// 実行した結果(計画だけのときは、予定の説明)。
    pub message: String,
    /// 実行した場合の成否。計画だけなら None。
    pub ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct OptReport {
    pub items: Vec<OptItem>,
    /// 補足(標準の最適化が有効か、権限が足りない、など)。
    pub notes: Vec<String>,
}

fn run(cmd: &str, args: &[&str]) -> (bool, String) {
    match Command::new(cmd).args(args).output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.trim().is_empty() {
                text.push_str(err.trim());
            }
            (o.status.success(), text.trim().to_string())
        }
        Err(e) => (false, format!("{cmd} を実行できません: {e}")),
    }
}

/// `fstrim --verbose` の1行(例: `/: 12.3 GiB (13214203904 bytes) trimmed on /dev/vda2`)から、(マウント先, 説明)。
pub fn parse_fstrim_line(line: &str) -> Option<(String, String)> {
    let (mount, rest) = line.split_once(": ")?;
    if !mount.starts_with('/') {
        return None;
    }
    Some((mount.to_string(), rest.trim().to_string()))
}

#[cfg(target_os = "linux")]
pub fn run_optimize(apply: bool) -> OptReport {
    let mut rep = OptReport::default();
    let args: Vec<&str> = if apply { vec!["--all", "--verbose"] } else { vec!["--all", "--verbose", "--dry-run"] };
    let (ok, text) = run("fstrim", &args);
    for line in text.lines() {
        if let Some((mount, msg)) = parse_fstrim_line(line) {
            rep.items.push(OptItem { target: mount, action: "TRIM".into(), message: msg, ok: if apply { Some(ok) } else { None } });
        }
    }
    if !ok {
        rep.notes.push(format!("fstrim が失敗しました(root 権限や、ディスクの TRIM 対応が必要): {}", text.lines().next().unwrap_or("")));
    } else if rep.items.is_empty() {
        rep.notes.push("TRIM できるファイルシステムがありませんでした(仮想ディスクで、TRIM が使えない場合があります)".into());
    }
    let (_, timers) = run("systemctl", &["is-enabled", "fstrim.timer"]);
    rep.notes.push(format!("OS 標準の定期 TRIM(fstrim.timer): {}", if timers.trim() == "enabled" { "有効" } else { "無効(この機能で、定期的に実行できます)" }));
    rep
}

#[cfg(target_os = "macos")]
pub fn run_optimize(_apply: bool) -> OptReport {
    OptReport { items: vec![], notes: vec!["macOS(APFS)は、自動で TRIM されるので、何もしません".into()] }
}

#[cfg(windows)]
pub fn run_optimize(apply: bool) -> OptReport {
    let mut rep = OptReport::default();
    // 各ドライブ文字が、SSD か HDD かを調べる。
    let list_script = r#"$ErrorActionPreference='SilentlyContinue'
foreach ($v in Get-Volume | Where-Object { $_.DriveLetter -and $_.DriveType -eq 'Fixed' }) {
  $p = Get-Partition -DriveLetter $v.DriveLetter
  $m = if ($p) { (Get-Disk -Number $p.DiskNumber | Get-PhysicalDisk).MediaType } else { '' }
  "$($v.DriveLetter)|$m"
}"#;
    let (_, text) = run("powershell", &["-NoProfile", "-Command", list_script]);
    for line in text.lines() {
        let Some((letter, media)) = line.trim().split_once('|') else { continue };
        let media = media.trim();
        let (action, ps_arg) = match media {
            "SSD" => ("ReTrim", "-ReTrim"),
            "HDD" => ("Defrag", "-Defrag"),
            _ => {
                rep.notes.push(format!("{letter}: の種類({media})が分からないので、最適化しません"));
                continue;
            }
        };
        if !apply {
            rep.items.push(OptItem { target: format!("{letter}:"), action: action.into(), message: format!("Optimize-Volume -DriveLetter {letter} {ps_arg} を実行する予定"), ok: None });
            continue;
        }
        let cmd = format!("Optimize-Volume -DriveLetter {letter} {ps_arg} -ErrorAction Stop; 'done'");
        let (ok, out) = run("powershell", &["-NoProfile", "-Command", &cmd]);
        let message = if ok {
            "完了".to_string()
        } else if out.contains("Access is denied") || out.contains("アクセスが拒否") || out.contains("administrator") || out.contains("管理者") {
            "管理者権限が必要です(管理者として実行してください)".to_string()
        } else {
            out.lines().next().unwrap_or("失敗").to_string()
        };
        rep.items.push(OptItem { target: format!("{letter}:"), action: action.into(), message, ok: Some(ok) });
    }
    let (ok, state) = run("powershell", &["-NoProfile", "-Command", "(Get-ScheduledTask -TaskName ScheduledDefrag -ErrorAction SilentlyContinue).State"]);
    if ok && !state.is_empty() {
        rep.notes.push(format!("Windows 標準の「ドライブの最適化」(ScheduledDefrag): {state}"));
    }
    rep
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn run_optimize(_apply: bool) -> OptReport {
    OptReport { items: vec![], notes: vec!["このOSの最適化には、対応していません".into()] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fstrim_verbose_lines() {
        let (m, d) = parse_fstrim_line("/: 12.3 GiB (13214203904 bytes) trimmed on /dev/vda2").unwrap();
        assert_eq!(m, "/");
        assert!(d.contains("trimmed"));
        let (m, _) = parse_fstrim_line("/boot: 0 B (0 bytes) trimmed on /dev/vda1").unwrap();
        assert_eq!(m, "/boot");
        assert!(parse_fstrim_line("fstrim: /x: FITRIM ioctl failed").is_none(), "マウント先で始まらない行は、読み飛ばす");
        assert!(parse_fstrim_line("").is_none());
    }
}
