use anyhow::Result;
use std::path::PathBuf;
use vps_tidy::{config, engine::Engine, report, sys};

const HELP: &str = "vps-tidy — VPSの無駄なデータを、固定の規則だけで、安全に整理する

使い方:
  vps-tidy [--config FILE] [--apply] [--heavy] [--no-report]

  --config FILE   設定ファイル(既定: /etc/vps-tidy/tidy.toml)
  --apply         実際に消す。付けなければ、試し実行(何も消さない)
  --heavy         ディスク使用率に関わらず、強めの整理(heavy = true の規則)も動かす
  --no-report     報告ファイルを書かない(画面に出すだけ)
  --version / --help
";

fn main() -> Result<()> {
    let mut cfg_path = PathBuf::from("/etc/vps-tidy/tidy.toml");
    let (mut apply, mut force_heavy, mut no_report) = (false, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("--config にはファイル名が要ります"))?),
            "--apply" => apply = true,
            "--heavy" => force_heavy = true,
            "--no-report" => no_report = true,
            "--version" => {
                println!("vps-tidy {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(());
            }
            other => {
                eprintln!("不明な引数: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    let cfg = match config::load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(2);
        }
    };

    let mount = cfg.heavy.mountpoint.clone();
    let used_before = sys::used_percent(&mount);
    let free_before = sys::free_bytes(&mount);
    let heavy_on = force_heavy || used_before.is_some_and(|u| u >= cfg.heavy.trigger_used_percent);

    let mut engine = Engine::new(&cfg, apply, heavy_on);
    engine.run();
    let summary = engine.summary();

    let used_after = sys::used_percent(&mount);
    let free_after = sys::free_bytes(&mount);
    let text = report::render(&report::Inputs {
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
    print!("{text}");
    if !no_report {
        match report::write(&cfg.report.dir, &text, cfg.report.keep_days) {
            Ok(p) => eprintln!("報告を書きました: {}", p.display()),
            Err(e) => eprintln!("報告を書けませんでした: {e}"),
        }
    }
    Ok(())
}
