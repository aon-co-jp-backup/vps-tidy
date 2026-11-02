//! 設定ファイル(TOML)。実際のパスは VPS の `/etc/vps-tidy/tidy.toml` にだけ置き、リポジトリには入れない
//! (`tidy.example.toml` は、一般的な例だけ)。

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub guard: Guard,
    #[serde(default)]
    pub heavy: Heavy,
    #[serde(default)]
    pub report: ReportCfg,
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
}

/// 安全装置。
#[derive(Debug, Clone, Deserialize)]
pub struct Guard {
    /// どの規則に一致しても、**絶対に触らない**パス(その下と、その上のフォルダ丸ごとの削除も拒否)。
    #[serde(default)]
    pub protected: Vec<PathBuf>,
    /// 1回の実行で消してよい量の上限(GB)。超える分は、次回に回す。
    #[serde(default = "default_cap")]
    pub max_delete_gb: f64,
    /// 実行ファイルを調べるサービス定義のフォルダ(`target` の `deps`/`debug` から動くものがあれば、整理しない)。
    #[serde(default = "default_unit_dirs")]
    pub unit_dirs: Vec<PathBuf>,
}

impl Default for Guard {
    fn default() -> Self {
        Self { protected: vec![], max_delete_gb: default_cap(), unit_dirs: default_unit_dirs() }
    }
}

fn default_cap() -> f64 {
    30.0
}

fn default_unit_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("/etc/systemd/system")]
}

/// 「強めの整理」(`heavy = true` の規則)を動かす条件。
#[derive(Debug, Clone, Deserialize)]
pub struct Heavy {
    /// このディスク使用率(%)以上のときだけ、強めの規則を動かす。
    #[serde(default = "default_trigger")]
    pub trigger_used_percent: f64,
    #[serde(default = "default_mount")]
    pub mountpoint: String,
}

impl Default for Heavy {
    fn default() -> Self {
        Self { trigger_used_percent: default_trigger(), mountpoint: default_mount() }
    }
}

fn default_trigger() -> f64 {
    75.0
}

fn default_mount() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportCfg {
    #[serde(default = "default_report_dir")]
    pub dir: PathBuf,
    #[serde(default = "default_keep_days")]
    pub keep_days: u64,
}

impl Default for ReportCfg {
    fn default() -> Self {
        Self { dir: default_report_dir(), keep_days: default_keep_days() }
    }
}

fn default_report_dir() -> PathBuf {
    PathBuf::from("/var/log/vps-tidy")
}

fn default_keep_days() -> u64 {
    60
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// 古いファイルの削除(古い実行ファイルの控え、一時ファイルなど)。`keep_latest` 個は、新しい順に残す。
    OldFiles,
    /// 古いログの圧縮(gzip)。
    CompressLogs,
    /// キャッシュフォルダの削除(フォルダごと)。
    CacheDir,
    /// cargo の `target` から、中間生成物だけを消す(最終バイナリは残す)。
    CargoSlim,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub name: String,
    pub kind: Kind,
    /// true なら「強めの整理」。ディスク使用率が `heavy.trigger_used_percent` 以上のときだけ動かす。
    #[serde(default)]
    pub heavy: bool,
    /// 対象のパターン(`*` と `**` が使える)。
    pub globs: Vec<String>,
    /// これより新しいものは、触らない(日)。
    #[serde(default)]
    pub min_age_days: f64,
    /// `old_files` で、同じ種類のうち、新しい順にこの個数は残す。
    #[serde(default)]
    pub keep_latest: usize,
    /// `cargo_slim` で、この時間以内に更新されている `target` は、ビルド中かもしれないので触らない。
    #[serde(default = "default_untouched")]
    pub untouched_hours: f64,
}

fn default_untouched() -> f64 {
    24.0
}

pub fn load(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("設定ファイルを読めません: {}", path.display()))?;
    let cfg: Config = toml::from_str(&text).with_context(|| format!("設定ファイルの書き方が正しくありません: {}", path.display()))?;
    for r in &cfg.rules {
        if r.globs.is_empty() {
            anyhow::bail!("規則「{}」に globs がありません", r.name);
        }
    }
    Ok(cfg)
}
