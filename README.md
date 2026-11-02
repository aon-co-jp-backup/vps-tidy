# vps-tidy

VPS(Linux)と手元の PC(Windows)の**無駄なデータ**(古い実行ファイルの控え、古いログ、キャッシュ、cargo のビルドの残りかす、`node_modules` など)を、**固定の規則だけ**で、安全に、定期的に整理する Rust 製のツール。

*Safely tidies wasteful data on a Linux VPS and a Windows PC (old binary backups, old logs, caches, cargo build leftovers, `node_modules`) on a schedule, using **fixed rules only**. Dry-run by default.*

## 考え方 / Design

- **消す対象は、設定の規則だけで決める。AI は使わない。** 消す・消さないの判断を、毎回変わりうる推測に任せると、事故のときに説明できないため。報告は、規則の結果から決まった形で組み立てる。
  *What to delete is decided by fixed rules, never by an AI: a guess that can change from run to run is not acceptable for deletion. The report is built from the rules' results.*
- **既定は試し実行。** `--apply` を付けたときだけ、実際に消す。
- **すべての削除が、同じ安全装置を通る**:
  1. **保護パス**(`guard.protected`)の下、または、それを**含む**フォルダは、どの規則でも触らない。
  2. **使用中**のファイル(プロセスが開いている、実行中)は消さない。
  3. **1回の削除の上限**(`guard.max_delete_gb`)。超えた分は次回に回す。
  4. **強めの整理**(`heavy = true`)は、ディスク使用率が基準(`heavy.trigger_used_percent`)以上のときだけ。
  5. `cargo_slim` は、さらに: `CACHEDIR.TAG` がある `target` だけ / `release` の直下に最終バイナリが残る / `cargo`・`rustc` が動いていない / 指定時間以内に更新されていない / サービス定義が `deps`・`debug` を指していない、の**すべて**を満たすときだけ。**最終バイナリには触らない。**

## 規則の種類 / Rule kinds

| `kind` | 内容 |
|---|---|
| `old_files` | 古いファイルの削除。同じ種類の控え(`.bak`・`.before-*`)は、新しい順に `keep_latest` 個を残す。`min_age_days` より新しいものは触らない。 |
| `compress_logs` | 古いログを gzip で圧縮(削除はしない)。 |
| `cache_dir` | キャッシュのフォルダを、フォルダごと削除。 |
| `cargo_slim` | cargo の `target` から、中間生成物(`release/deps`・`build`・`.fingerprint`・`incremental`・`examples`、`debug`)だけを消す(最終バイナリは残す。動いているサービスの `target` 向け)。 |
| `regen_dirs` | 再生成できるフォルダ(`target`・`node_modules`・`.gradle`・`__pycache__`)を、**フォルダごと**消す。「再生成できる」目印が必要: `marker_file`(中にあるファイル。例: `CACHEDIR.TAG`)、`parent_marker_any`(隣にあるファイルのどれか。例: `package-lock.json`)。`min_age_days` 以内に使われた(2階層下まで見て更新があった)ものは触らない。 |

## Windows(手元の PC)/ Windows

- **使用中の確認:** Windows では、開いているファイルの一覧が取れないので、**消す前に、フォルダの名前を変えられるか**(中で使われていれば失敗する)で確かめる。使用中のものは、見送る。
- **最近使っているか:** フォルダ自身と、2階層下までの項目の更新時刻のうち、最も新しいものを見る(`target` は、最近のビルドが `target/debug/deps` などに現れる)。
- **既定の場所:** 設定 `%LOCALAPPDATA%\vps-tidy\tidy.toml`、報告 `%LOCALAPPDATA%\vps-tidy\reports\`。設定の例は [`tidy.example.windows.toml`](tidy.example.windows.toml)。
- **定期実行:** [`deploy/windows/register-task.ps1`](deploy/windows/register-task.ps1) が、毎週1回のタスクスケジューラを登録する(`-Apply` を付けるまでは、試し実行で、報告だけ書く)。
- 同期ソフト(例: SyncFolder)が、別のドライブへミラーしている場合は、**片方だけ消しても、同期で戻る**ことがある。`target` は、同期の対象から外すのが、根本の対策。

## 使い方 / Usage

```bash
vps-tidy --config /etc/vps-tidy/tidy.toml            # 試し実行(何も消さない)
vps-tidy --config /etc/vps-tidy/tidy.toml --apply    # 実際に消す
vps-tidy --config /etc/vps-tidy/tidy.toml --heavy    # 使用率に関わらず、強めの整理も動かす
```

設定の書き方は [`tidy.example.toml`](tidy.example.toml)。**実際の設定(パスや構成が分かるもの)は、サーバーの `/etc/vps-tidy/tidy.toml` にだけ置き、リポジトリには入れない。**

定期実行は、`deploy/vps-tidy.service` と `deploy/vps-tidy.timer`(毎日 04:40 JST、最低の優先度)。報告は `/var/log/vps-tidy/` に残る(60日)。

## 導入の手順の例 / Install

```bash
# ビルド(サーバーのメモリが小さいときは、別のマシンで作って、バイナリだけ置く)
cargo build --release
install -m 755 target/release/vps-tidy /usr/local/bin/vps-tidy
install -d /etc/vps-tidy && cp tidy.example.toml /etc/vps-tidy/tidy.toml   # 編集する
cp deploy/vps-tidy.service deploy/vps-tidy.timer /etc/systemd/system/
systemctl daemon-reload
vps-tidy --config /etc/vps-tidy/tidy.toml            # まず、試し実行で、内容を確認する
systemctl enable --now vps-tidy.timer
```

## 正直な開示 / Honest notes

- Linux(`/proc`・`df`・`gzip`)と Windows(PowerShell の `Get-PSDrive`、`tasklist`)で動く。Windows には、`cargo_slim` のサービス定義の確認(`unit_dirs`)は無い。
- 「使用中」の判定は、実行時点で開いているファイルだけ。実行の直後に開くものまでは、防げない(だから、保護パスと、最終バイナリを残す設計にしている)。
- `cargo_slim` の後は、そのサービスを VPS 上で再ビルドすると、ゼロからの作り直しになる(別のマシンでビルドして、バイナリを置く運用が前提)。

## ライセンス / License

MIT OR Apache-2.0
