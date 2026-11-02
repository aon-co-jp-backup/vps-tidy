//! 「無駄なデータの整理」を1回実行する、共通の入口。コマンド(`vps-tidy`)と、組み込み先(`open-easy-web` のサーバー)の
//! 両方が、同じ処理を呼ぶ。

use crate::config::Config;
use crate::engine::{Action, Engine, Summary};
use crate::{report, sys};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TidyRun {
    pub applied: bool,
    pub heavy_on: bool,
    pub used_before: Option<f64>,
    pub used_after: Option<f64>,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    pub summary: Summary,
    /// 人が読む報告の文章(コマンドが画面に出すものと同じ)。
    pub report_text: String,
    #[serde(skip)]
    pub actions: Vec<Action>,
}

/// 整理を1回実行する。`apply` が false なら、試し実行(何も消さない)。
pub fn run_tidy(cfg: &Config, apply: bool, force_heavy: bool) -> TidyRun {
    let mount = cfg.heavy.mountpoint.clone();
    let used_before = sys::used_percent(&mount);
    let free_before = sys::free_bytes(&mount);
    let heavy_on = force_heavy || used_before.is_some_and(|u| u >= cfg.heavy.trigger_used_percent);

    let mut engine = Engine::new(cfg, apply, heavy_on);
    engine.run();
    let summary = engine.summary();

    let used_after = sys::used_percent(&mount);
    let free_after = sys::free_bytes(&mount);
    let report_text = report::render(&report::Inputs {
        apply,
        heavy_on,
        trigger: cfg.heavy.trigger_used_percent,
        used_before,
        used_after,
        free_before,
        free_after,
        actions: &engine.actions,
        summary: &summary,
        cap_gb: cfg.guard.max_delete_gb,
    });
    TidyRun { applied: apply, heavy_on, used_before, used_after, free_before, free_after, summary, report_text, actions: engine.actions }
}
