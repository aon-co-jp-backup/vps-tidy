use anyhow::Result;
use std::path::PathBuf;
use vps_tidy::{config, health, optimize, report, runner};

const HELP: &str = "vps-tidy — ストレージ(HDD/SSD)を、固定の規則だけで、安全に定期メンテナンスする

使い方:
  vps-tidy [tidy]  [--config FILE] [--apply] [--heavy] [--no-report]   無駄なデータの整理(既定)
  vps-tidy health  [--json]                                           ディスクの健康チェック(SMART など)
  vps-tidy optimize [--apply]                                         SSD の TRIM / HDD の最適化

  --config FILE   設定ファイル(既定: Linux は /etc/vps-tidy/tidy.toml、Windows は %LOCALAPPDATA%\\vps-tidy\\tidy.toml)
  --apply         実際に行う。付けなければ、試し実行(何もしない)
  --heavy         ディスク使用率に関わらず、強めの整理(heavy = true の規則)も動かす
  --no-report     報告ファイルを書かない(画面に出すだけ)
  --json          health の結果を、JSON で出す
  --version / --help

health は、悪い状態(注意・危険)があると、終了コード 1(注意)/ 2(危険)で終わる。
";

fn status_label(s: health::Status) -> &'static str {
    match s {
        health::Status::Ok => "正常",
        health::Status::Unknown => "不明",
        health::Status::Warning => "注意",
        health::Status::Critical => "危険",
    }
}

fn run_health(json: bool) -> i32 {
    let disks = health::collect();
    let worst = health::worst(&disks);
    if json {
        match serde_json::to_string_pretty(&disks) {
            Ok(t) => println!("{t}"),
            Err(e) => eprintln!("JSON にできませんでした: {e}"),
        }
    } else {
        println!("ディスクの健康チェック  全体: {}", status_label(worst));
        for d in &disks {
            println!("\n[{}] {}  {}  ({:?})", status_label(d.status), d.name, d.model, d.kind);
            for r in &d.reasons {
                println!("    - {r}");
            }
            let mut facts = vec![];
            if let Some(t) = d.temperature_c {
                facts.push(format!("温度 {t}℃"));
            }
            if let Some(h) = d.power_on_hours {
                facts.push(format!("通電 {h} 時間"));
            }
            if let Some(w) = d.wear_percent_used {
                facts.push(format!("消耗 {w}%"));
            }
            if !facts.is_empty() {
                println!("    ({})", facts.join(" / "));
            }
        }
    }
    match worst {
        health::Status::Critical => 2,
        health::Status::Warning => 1,
        _ => 0,
    }
}

fn run_optimize(apply: bool) {
    let rep = optimize::run_optimize(apply);
    println!("最適化 {}", if apply { "(実行)" } else { "(計画だけ。実行するには --apply)" });
    for i in &rep.items {
        let st = match i.ok {
            Some(true) => "成功",
            Some(false) => "失敗",
            None => "予定",
        };
        println!("  [{st}] {}  {}  {}", i.target, i.action, i.message);
    }
    for n in &rep.notes {
        println!("  * {n}");
    }
}

fn main() -> Result<()> {
    let mut cfg_path: PathBuf = config::default_config_path();
    let (mut apply, mut force_heavy, mut no_report) = (false, false, false);
    let mut args = std::env::args().skip(1).peekable();
    let mut sub = "tidy";
    if let Some(first) = args.peek() {
        match first.as_str() {
            "tidy" | "health" | "optimize" => {
                sub = match first.as_str() {
                    "health" => "health",
                    "optimize" => "optimize",
                    _ => "tidy",
                };
                args.next();
            }
            _ => {}
        }
    }
    let mut json = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("--config にはファイル名が要ります"))?),
            "--apply" => apply = true,
            "--heavy" => force_heavy = true,
            "--no-report" => no_report = true,
            "--json" => json = true,
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

    match sub {
        "health" => std::process::exit(run_health(json)),
        "optimize" => {
            run_optimize(apply);
            return Ok(());
        }
        _ => {}
    }

    let cfg = match config::load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(2);
        }
    };

    let run = runner::run_tidy(&cfg, apply, force_heavy);
    let text = run.report_text.clone();
    print!("{text}");
    if !no_report {
        match report::write(&cfg.report.dir, &text, cfg.report.keep_days) {
            Ok(p) => eprintln!("報告を書きました: {}", p.display()),
            Err(e) => eprintln!("報告を書けませんでした: {e}"),
        }
    }
    Ok(())
}
