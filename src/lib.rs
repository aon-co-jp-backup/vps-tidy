//! vps-tidy: VPS・PCのストレージ(HDD/SSD)を、固定の規則だけで、安全に定期メンテナンスする。
//! 無駄なデータの整理(`engine`・`runner`)、SMART の健康チェック(`health`)、TRIM/最適化(`optimize`)。
//!
//! - 既定は**試し実行**(`--apply` を付けたときだけ、実際に消す)。
//! - 消す対象は、設定の規則だけで決める(AIは使わない)。
//! - すべての削除は、同じ安全装置(保護パス・使用中・1回の上限・強めの整理の条件)を通る。

pub mod config;
pub mod engine;
pub mod health;
pub mod optimize;
pub mod report;
pub mod runner;
pub mod sys;
