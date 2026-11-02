//! ディスクの健康チェック(SMART など)。
//!
//! - Linux/macOS: `smartctl --json`(smartmontools)。無ければ、「取得できない」と正直に報告する。
//! - Windows: PowerShell の `Get-PhysicalDisk` と `Get-StorageReliabilityCounter`(追加のソフト不要)。
//!
//! 判定(`Status`)は、取得した値から決まる**純粋な規則**で、AI は使わない。しきい値は、下の定数。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    /// 取得できなかった(SMART が無効、仮想ディスク、ツールが無い など)。「正常」ではない。
    Unknown,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Hdd,
    Ssd,
    Nvme,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskHealth {
    pub name: String,
    pub model: String,
    pub kind: Kind,
    pub size_bytes: Option<u64>,
    pub status: Status,
    /// 判定の理由(人が読む文章)。
    pub reasons: Vec<String>,
    pub temperature_c: Option<i32>,
    pub power_on_hours: Option<u64>,
    pub reallocated_sectors: Option<u64>,
    pub pending_sectors: Option<u64>,
    pub uncorrectable_sectors: Option<u64>,
    /// SSD/NVMe の消耗(%)。100 に近いほど寿命。
    pub wear_percent_used: Option<u32>,
    /// どこから取った情報か(`smartctl` / `windows-storage`)。
    pub source: &'static str,
}

// ---- しきい値 ---------------------------------------------------------------
const TEMP_WARN: i32 = 55; // これ以上: 注意(HDD は 55℃ 超が長く続くと寿命が縮む)
const TEMP_CRIT: i32 = 70; // これ以上: 危険
const REALLOCATED_CRIT: u64 = 50; // 代替処理済みセクターが、これ以上: 危険(1 以上は注意)
const WEAR_WARN: u32 = 80;
const WEAR_CRIT: u32 = 95;

impl DiskHealth {
    fn new(name: String, model: String, kind: Kind, source: &'static str) -> Self {
        Self {
            name,
            model,
            kind,
            size_bytes: None,
            status: Status::Ok,
            reasons: vec![],
            temperature_c: None,
            power_on_hours: None,
            reallocated_sectors: None,
            pending_sectors: None,
            uncorrectable_sectors: None,
            wear_percent_used: None,
            source,
        }
    }

    fn raise(&mut self, s: Status, reason: impl Into<String>) {
        if s > self.status {
            self.status = s;
        }
        self.reasons.push(reason.into());
    }

    /// 取得した値から、状態を決める(共通の規則)。`smart_passed` は、SMART の自己診断の総合結果。
    fn judge(&mut self, smart_passed: Option<bool>) {
        match smart_passed {
            Some(false) => self.raise(Status::Critical, "SMART の自己診断が「失敗」です(故障が近い可能性が高い)"),
            Some(true) => {}
            None => {
                if self.status == Status::Ok {
                    self.status = Status::Unknown;
                }
                self.reasons.push("SMART の総合結果を取得できませんでした".to_string());
            }
        }
        if let Some(n) = self.reallocated_sectors {
            if n >= REALLOCATED_CRIT {
                self.raise(Status::Critical, format!("代替処理済みのセクターが {n} 個(多い)"));
            } else if n > 0 {
                self.raise(Status::Warning, format!("代替処理済みのセクターが {n} 個(増え続けるなら交換を検討)"));
            }
        }
        if let Some(n) = self.pending_sectors {
            if n > 0 {
                self.raise(Status::Warning, format!("読めずに保留中のセクターが {n} 個"));
            }
        }
        if let Some(n) = self.uncorrectable_sectors {
            if n > 0 {
                self.raise(Status::Warning, format!("訂正できなかったセクターが {n} 個"));
            }
        }
        if let Some(w) = self.wear_percent_used {
            if w >= WEAR_CRIT {
                self.raise(Status::Critical, format!("SSD の消耗が {w}%(寿命が近い)"));
            } else if w >= WEAR_WARN {
                self.raise(Status::Warning, format!("SSD の消耗が {w}%"));
            }
        }
        if let Some(t) = self.temperature_c {
            if t >= TEMP_CRIT {
                self.raise(Status::Critical, format!("温度が {t}℃(高すぎる)"));
            } else if t >= TEMP_WARN {
                self.raise(Status::Warning, format!("温度が {t}℃(高め)"));
            }
        }
        if self.status == Status::Ok {
            self.reasons.push("問題は見つかりませんでした".to_string());
        }
    }
}

/// 全体のうち、最も悪い状態。
pub fn worst(disks: &[DiskHealth]) -> Status {
    disks.iter().map(|d| d.status).max().unwrap_or(Status::Unknown)
}

fn limits() -> rust_json::Limits {
    rust_json::Limits { max_bytes: 4 * 1024 * 1024, max_depth: 16, reject_duplicate_keys: false }
}

// ---- smartctl(Linux/macOS)-------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct SmartctlOut {
    #[serde(default)]
    device: Option<SmartDevice>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    rotation_rate: Option<u32>,
    #[serde(default)]
    user_capacity: Option<Capacity>,
    #[serde(default)]
    smart_status: Option<SmartStatus>,
    #[serde(default)]
    temperature: Option<Temperature>,
    #[serde(default)]
    power_on_time: Option<PowerOn>,
    #[serde(default)]
    ata_smart_attributes: Option<AtaAttrs>,
    #[serde(default)]
    nvme_smart_health_information_log: Option<NvmeLog>,
}

#[derive(Debug, Deserialize, Default)]
struct SmartDevice {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct Capacity {
    #[serde(default)]
    bytes: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct SmartStatus {
    #[serde(default)]
    passed: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct Temperature {
    #[serde(default)]
    current: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct PowerOn {
    #[serde(default)]
    hours: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct AtaAttrs {
    #[serde(default)]
    table: Vec<AtaAttr>,
}

#[derive(Debug, Deserialize)]
struct AtaAttr {
    id: u32,
    #[serde(default)]
    value: u32,
    #[serde(default)]
    raw: AttrRaw,
}

#[derive(Debug, Deserialize, Default)]
struct AttrRaw {
    #[serde(default)]
    value: u64,
}

#[derive(Debug, Deserialize, Default)]
struct NvmeLog {
    #[serde(default)]
    critical_warning: u64,
    #[serde(default)]
    temperature: Option<i32>,
    #[serde(default)]
    available_spare: Option<u32>,
    #[serde(default)]
    available_spare_threshold: Option<u32>,
    #[serde(default)]
    percentage_used: Option<u32>,
    #[serde(default)]
    media_errors: u64,
    #[serde(default)]
    power_on_hours: Option<u64>,
}

/// `smartctl --json -a <dev>` の出力から、1台の状態を作る。
pub fn parse_smartctl(json: &str) -> Result<DiskHealth> {
    let o: SmartctlOut = rust_json::from_str_secure(json, &limits()).map_err(|e| anyhow::anyhow!("smartctl の出力を読めません: {e}"))?;
    let name = o.device.map(|d| d.name).unwrap_or_default();
    let model = o.model_name.unwrap_or_else(|| "(不明)".to_string());
    let kind = if o.nvme_smart_health_information_log.is_some() {
        Kind::Nvme
    } else {
        match o.rotation_rate {
            Some(0) => Kind::Ssd,
            Some(_) => Kind::Hdd,
            None => Kind::Unknown,
        }
    };
    let mut d = DiskHealth::new(name, model, kind, "smartctl");
    d.size_bytes = o.user_capacity.and_then(|c| c.bytes);
    d.temperature_c = o.temperature.and_then(|t| t.current);
    d.power_on_hours = o.power_on_time.and_then(|p| p.hours);
    let passed = o.smart_status.and_then(|s| s.passed);

    if let Some(n) = o.nvme_smart_health_information_log {
        d.temperature_c = d.temperature_c.or(n.temperature);
        d.power_on_hours = d.power_on_hours.or(n.power_on_hours);
        d.wear_percent_used = n.percentage_used;
        if n.critical_warning != 0 {
            d.raise(Status::Critical, format!("NVMe の重大な警告フラグが立っています(値 {})", n.critical_warning));
        }
        if n.media_errors > 0 {
            d.raise(Status::Warning, format!("メディアエラーが {} 件", n.media_errors));
        }
        if let (Some(s), Some(t)) = (n.available_spare, n.available_spare_threshold) {
            if s <= t {
                d.raise(Status::Critical, format!("予備領域が {s}%(しきい値 {t}% 以下)"));
            }
        }
    }
    if let Some(a) = o.ata_smart_attributes {
        for at in &a.table {
            match at.id {
                5 => d.reallocated_sectors = Some(at.raw.value),
                197 => d.pending_sectors = Some(at.raw.value),
                198 => d.uncorrectable_sectors = Some(at.raw.value),
                // SSD の消耗(正規化値 100 から減っていく): 177 Wear_Leveling_Count / 231 SSD_Life_Left / 233 Media_Wearout_Indicator
                177 | 231 | 233 if at.value > 0 && at.value <= 100 => d.wear_percent_used = Some(100 - at.value),
                _ => {}
            }
        }
    }
    d.judge(passed);
    Ok(d)
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("{cmd} を実行できません"))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct ScanOut {
    #[serde(default)]
    devices: Vec<ScanDev>,
}

#[cfg(not(windows))]
#[derive(Debug, Deserialize)]
struct ScanDev {
    name: String,
}

#[cfg(not(windows))]
pub fn collect() -> Vec<DiskHealth> {
    let scan = match run("smartctl", &["--scan", "--json"]) {
        Ok(s) => s,
        Err(_) => {
            return vec![unavailable("smartctl が見つかりません(smartmontools を入れると、SMART を取得できます)")];
        }
    };
    let devs: Vec<String> = rust_json::from_str_secure::<ScanOut>(&scan, &limits()).map(|s| s.devices.into_iter().map(|d| d.name).collect()).unwrap_or_default();
    if devs.is_empty() {
        return vec![unavailable("SMART を読めるディスクが見つかりません(仮想ディスクでは、読めないことが多い)")];
    }
    let mut v = Vec::new();
    for d in devs {
        match run("smartctl", &["--json", "-a", &d]).and_then(|j| parse_smartctl(&j)) {
            Ok(mut h) => {
                if h.name.is_empty() {
                    h.name = d;
                }
                v.push(h);
            }
            Err(e) => v.push(unavailable_named(&d, format!("{e}"))),
        }
    }
    v
}

fn unavailable(msg: &str) -> DiskHealth {
    unavailable_named("(全体)", msg.to_string())
}

fn unavailable_named(name: &str, msg: String) -> DiskHealth {
    let mut d = DiskHealth::new(name.to_string(), "(不明)".to_string(), Kind::Unknown, "none");
    d.status = Status::Unknown;
    d.reasons.push(msg);
    d
}

// ---- Windows ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WinDisk {
    #[serde(default)]
    friendly_name: String,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    bus_type: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    health_status: Option<String>,
    #[serde(default)]
    temperature: Option<i32>,
    #[serde(default)]
    wear: Option<u32>,
    #[serde(default)]
    read_errors_uncorrected: Option<u64>,
    #[serde(default)]
    write_errors_uncorrected: Option<u64>,
    #[serde(default)]
    power_on_hours: Option<u64>,
}

/// PowerShell の出力(配列、または、1台だけのときは単体のオブジェクト)から、状態を作る。
pub fn parse_windows_disks(json: &str) -> Result<Vec<DiskHealth>> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }
    let disks: Vec<WinDisk> = if trimmed.starts_with('[') {
        rust_json::from_str_secure(trimmed, &limits()).map_err(|e| anyhow::anyhow!("ディスク情報を読めません: {e}"))?
    } else {
        vec![rust_json::from_str_secure(trimmed, &limits()).map_err(|e| anyhow::anyhow!("ディスク情報を読めません: {e}"))?]
    };
    Ok(disks
        .into_iter()
        .map(|w| {
            let bus = w.bus_type.clone().unwrap_or_default();
            let kind = match (w.media_type.as_deref(), bus.as_str()) {
                (_, "NVMe") => Kind::Nvme,
                (Some("SSD"), _) => Kind::Ssd,
                (Some("HDD"), _) => Kind::Hdd,
                _ => Kind::Unknown,
            };
            let mut d = DiskHealth::new(w.friendly_name.clone(), w.friendly_name, kind, "windows-storage");
            d.size_bytes = w.size;
            d.temperature_c = w.temperature.filter(|t| *t > 0);
            d.power_on_hours = w.power_on_hours;
            d.wear_percent_used = w.wear;
            d.uncorrectable_sectors = match (w.read_errors_uncorrected, w.write_errors_uncorrected) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
            };
            // Windows の総合判定(HealthStatus)を、SMART の自己診断の代わりに使う。
            let passed = match w.health_status.as_deref() {
                Some("Healthy") => Some(true),
                Some("Warning") => {
                    d.raise(Status::Warning, "Windows が「警告」と判定しています");
                    Some(true)
                }
                Some("Unhealthy") => Some(false),
                _ => None,
            };
            d.judge(passed);
            d
        })
        .collect())
}

#[cfg(windows)]
pub fn collect() -> Vec<DiskHealth> {
    // 値は、文字列にしてから JSON にする(列挙型が数値で出るのを避ける)。管理者権限が無くても、多くの項目は読める。
    let script = r#"$ErrorActionPreference='SilentlyContinue'
$r = foreach ($d in Get-PhysicalDisk) {
  $c = $d | Get-StorageReliabilityCounter
  [pscustomobject]@{
    FriendlyName = [string]$d.FriendlyName
    MediaType = [string]$d.MediaType
    BusType = [string]$d.BusType
    Size = [uint64]$d.Size
    HealthStatus = [string]$d.HealthStatus
    Temperature = $c.Temperature
    Wear = $c.Wear
    ReadErrorsUncorrected = $c.ReadErrorsUncorrected
    WriteErrorsUncorrected = $c.WriteErrorsUncorrected
    PowerOnHours = $c.PowerOnHours
  }
}
@($r) | ConvertTo-Json -Compress"#;
    match run("powershell", &["-NoProfile", "-Command", script]).and_then(|j| parse_windows_disks(&j)) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => vec![unavailable("ディスク情報を取得できませんでした")],
        Err(e) => vec![unavailable(&format!("{e}"))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEALTHY_HDD: &str = r#"{"device":{"name":"/dev/sda"},"model_name":"WDC WD30EZRX","rotation_rate":5400,
      "user_capacity":{"bytes":3000592982016},"smart_status":{"passed":true},"temperature":{"current":38},
      "power_on_time":{"hours":41000},
      "ata_smart_attributes":{"table":[
        {"id":5,"value":200,"raw":{"value":0}},{"id":197,"value":200,"raw":{"value":0}},{"id":198,"value":200,"raw":{"value":0}}]}}"#;

    #[test]
    fn a_healthy_hdd_is_ok() {
        let d = parse_smartctl(HEALTHY_HDD).unwrap();
        assert_eq!(d.status, Status::Ok);
        assert_eq!(d.kind, Kind::Hdd);
        assert_eq!(d.model, "WDC WD30EZRX");
        assert_eq!(d.temperature_c, Some(38));
        assert_eq!(d.power_on_hours, Some(41000));
    }

    #[test]
    fn reallocated_pending_and_failed_self_test_raise_the_status() {
        let warn = HEALTHY_HDD.replace(r#"{"id":5,"value":200,"raw":{"value":0}}"#, r#"{"id":5,"value":190,"raw":{"value":8}}"#);
        assert_eq!(parse_smartctl(&warn).unwrap().status, Status::Warning);
        let crit = HEALTHY_HDD.replace(r#"{"id":5,"value":200,"raw":{"value":0}}"#, r#"{"id":5,"value":100,"raw":{"value":120}}"#);
        assert_eq!(parse_smartctl(&crit).unwrap().status, Status::Critical);
        let pend = HEALTHY_HDD.replace(r#"{"id":197,"value":200,"raw":{"value":0}}"#, r#"{"id":197,"value":200,"raw":{"value":3}}"#);
        assert_eq!(parse_smartctl(&pend).unwrap().status, Status::Warning);
        let failed = HEALTHY_HDD.replace(r#""passed":true"#, r#""passed":false"#);
        assert_eq!(parse_smartctl(&failed).unwrap().status, Status::Critical);
    }

    #[test]
    fn temperature_thresholds() {
        let hot = HEALTHY_HDD.replace(r#""current":38"#, r#""current":58"#);
        assert_eq!(parse_smartctl(&hot).unwrap().status, Status::Warning);
        let very_hot = HEALTHY_HDD.replace(r#""current":38"#, r#""current":72"#);
        assert_eq!(parse_smartctl(&very_hot).unwrap().status, Status::Critical);
    }

    #[test]
    fn nvme_wear_spare_and_critical_warning() {
        let nvme = |pct: u32, spare: u32, crit: u32| {
            format!(
                r#"{{"device":{{"name":"/dev/nvme0"}},"model_name":"Samsung 970","smart_status":{{"passed":true}},
                "nvme_smart_health_information_log":{{"critical_warning":{crit},"temperature":40,"available_spare":{spare},
                "available_spare_threshold":10,"percentage_used":{pct},"media_errors":0,"power_on_hours":900}}}}"#
            )
        };
        assert_eq!(parse_smartctl(&nvme(5, 100, 0)).unwrap().status, Status::Ok);
        assert_eq!(parse_smartctl(&nvme(85, 100, 0)).unwrap().status, Status::Warning);
        assert_eq!(parse_smartctl(&nvme(97, 100, 0)).unwrap().status, Status::Critical);
        assert_eq!(parse_smartctl(&nvme(5, 8, 0)).unwrap().status, Status::Critical, "予備領域がしきい値以下");
        assert_eq!(parse_smartctl(&nvme(5, 100, 4)).unwrap().status, Status::Critical, "重大な警告フラグ");
        assert_eq!(parse_smartctl(&nvme(5, 100, 0)).unwrap().kind, Kind::Nvme);
    }

    #[test]
    fn ata_ssd_wear_comes_from_the_normalized_value() {
        let ssd = r#"{"device":{"name":"/dev/sdb"},"model_name":"Crucial MX500","rotation_rate":0,"smart_status":{"passed":true},
          "ata_smart_attributes":{"table":[{"id":5,"value":100,"raw":{"value":0}},{"id":177,"value":12,"raw":{"value":0}}]}}"#;
        let d = parse_smartctl(ssd).unwrap();
        assert_eq!(d.kind, Kind::Ssd);
        assert_eq!(d.wear_percent_used, Some(88));
        assert_eq!(d.status, Status::Warning);
    }

    #[test]
    fn a_missing_smart_status_is_unknown_not_ok() {
        let virt = r#"{"device":{"name":"/dev/vda"},"model_name":"QEMU HARDDISK"}"#;
        let d = parse_smartctl(virt).unwrap();
        assert_eq!(d.status, Status::Unknown, "取得できなかったものを、正常とは言わない");
    }

    #[test]
    fn windows_disks_array_and_single_object() {
        let arr = r#"[{"FriendlyName":"WD30EZRX","MediaType":"HDD","BusType":"SATA","Size":3000592982016,"HealthStatus":"Healthy","Temperature":36,"Wear":null,"ReadErrorsUncorrected":0,"WriteErrorsUncorrected":0,"PowerOnHours":41000},
          {"FriendlyName":"Samsung SSD","MediaType":"SSD","BusType":"NVMe","Size":500107862016,"HealthStatus":"Healthy","Temperature":41,"Wear":3,"ReadErrorsUncorrected":0,"WriteErrorsUncorrected":0,"PowerOnHours":900}]"#;
        let v = parse_windows_disks(arr).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].kind, Kind::Hdd);
        assert_eq!(v[1].kind, Kind::Nvme);
        assert_eq!(v[1].wear_percent_used, Some(3));
        assert!(v.iter().all(|d| d.status == Status::Ok));
        let one = r#"{"FriendlyName":"X","MediaType":"HDD","BusType":"SATA","HealthStatus":"Unhealthy"}"#;
        assert_eq!(parse_windows_disks(one).unwrap()[0].status, Status::Critical);
        let warn = r#"{"FriendlyName":"X","MediaType":"HDD","BusType":"SATA","HealthStatus":"Warning"}"#;
        assert_eq!(parse_windows_disks(warn).unwrap()[0].status, Status::Warning);
        let errs = r#"{"FriendlyName":"X","MediaType":"HDD","BusType":"SATA","HealthStatus":"Healthy","ReadErrorsUncorrected":2,"WriteErrorsUncorrected":1}"#;
        let d = &parse_windows_disks(errs).unwrap()[0];
        assert_eq!(d.uncorrectable_sectors, Some(3));
        assert_eq!(d.status, Status::Warning);
    }

    #[test]
    fn worst_picks_the_most_serious() {
        let a = parse_smartctl(HEALTHY_HDD).unwrap();
        let b = parse_smartctl(&HEALTHY_HDD.replace(r#""passed":true"#, r#""passed":false"#)).unwrap();
        assert_eq!(worst(&[a, b]), Status::Critical);
        assert_eq!(worst(&[]), Status::Unknown);
    }
}
