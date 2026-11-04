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

## API 方針: REST 全廃・gRPC / Connect へ (2026-11-04)

### 日本語
- **REST は全廃する。新しい API は REST で作らない。** ブラウザや `curl` から呼ぶ API は **Connect の単純呼び出し**(`POST /<パッケージ>.<Service>/<Method>`、`content-type: application/json`、本文は JSON)、サービス間は **gRPC / gRPC-web**(RPoem の管理API `open_runo.admin.v1.Admin` と同じ流儀)。ストリーミングは、必要になってから。
- **既存の REST は、触るときに順に移す。** 移したら旧 REST の経路は消す。移行が終わるまで残るが、**新規には追加しない**。
- **JSON は RS-JSON に統一**する(読み込みは `from_str_secure` / `parse_secure` でサイズ・深さ・重複キーの上限つき、書き出しは `rust_json::fast`)。ブラウザの WASM に `prost` は入れない(サイズが増えるため)。
- **例外(自分たちの API ではなく、相手の規格が決めているもの)**: ACME、OAuth/OIDC のコールバック、第三者(GitHub など)からの Webhook、ヘルスチェック、静的ファイル配信、OTLP など。これらは規格どおりの HTTP を受ける。
- 参考実装: `RPoem`(`open_runo.admin.v1.Admin`)、`open-easy-web`(`server/src/connect.rs`、`proto/open_easy_web/admin/v1/admin.proto`)。
- ※ このリポジトリのコードが既に方針どおりかどうかは、各リポジトリの現状による。この節は方針の記録であり、コードの変更を意味しない。

### English
- **REST is being abolished. New APIs are not built as REST.** APIs called from browsers or `curl` use **Connect unary calls** (`POST /<package>.<Service>/<Method>`, `content-type: application/json`, JSON body); service-to-service calls use **gRPC / gRPC-web** (the same style as RPoem's admin API `open_runo.admin.v1.Admin`). Streaming only when it is actually needed.
- **Existing REST endpoints are migrated one by one, when they are touched.** The old REST route is removed once migrated. Until migration is finished they remain, but **no new ones are added**.
- **JSON is unified on RS-JSON**: reading with `from_str_secure` / `parse_secure` (size, depth and duplicate-key limits), writing with `rust_json::fast`. `prost` is not put into browser WASM (it would grow the binary).
- **Exceptions (shapes dictated by someone else's standard, not our own API)**: ACME, OAuth/OIDC callbacks, webhooks from third parties (e.g. GitHub), health checks, static file serving, OTLP, and so on. These accept plain HTTP as the standard requires.
- Reference implementations: `RPoem` (`open_runo.admin.v1.Admin`) and `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Note: whether the code in a given repository already follows this policy depends on that repository. This section records the policy; it does not mean the code has been changed.

### Deutsch (KI-Übersetzung, noch nicht von Muttersprachlern geprüft)
- **REST wird abgeschafft. Neue APIs werden nicht als REST gebaut.** APIs, die aus dem Browser oder mit `curl` aufgerufen werden, nutzen **Connect-Unary-Aufrufe** (`POST /<Paket>.<Service>/<Methode>`, `content-type: application/json`, JSON-Body); zwischen Diensten gilt **gRPC / gRPC-web** (wie bei der Admin-API von RPoem, `open_runo.admin.v1.Admin`). Streaming erst, wenn es wirklich nötig ist.
- **Bestehende REST-Endpunkte werden einzeln migriert, sobald man sie anfasst.** Nach der Migration wird die alte REST-Route entfernt. Bis dahin bleiben sie bestehen, es werden aber **keine neuen hinzugefügt**.
- **JSON wird auf RS-JSON vereinheitlicht**: Lesen mit `from_str_secure` / `parse_secure` (Grenzen für Größe, Tiefe und doppelte Schlüssel), Schreiben mit `rust_json::fast`. `prost` kommt nicht in Browser-WASM (die Datei würde größer).
- **Ausnahmen (Formate, die ein fremder Standard vorgibt, nicht unsere eigene API)**: ACME, OAuth/OIDC-Callbacks, Webhooks von Dritten (z. B. GitHub), Health-Checks, statische Dateien, OTLP usw. Diese nehmen schlichtes HTTP entgegen, wie es der Standard verlangt.
- Referenzimplementierungen: `RPoem` (`open_runo.admin.v1.Admin`) und `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Hinweis: Ob der Code eines Repositorys diese Richtlinie schon befolgt, hängt vom jeweiligen Repository ab. Dieser Abschnitt hält die Richtlinie fest; er bedeutet nicht, dass der Code bereits geändert wurde.

### Русский (машинный перевод, носителями языка ещё не проверен)
- **REST полностью отменяется. Новые API не создаются как REST.** API, вызываемые из браузера или через `curl`, используют **унарные вызовы Connect** (`POST /<пакет>.<Service>/<Method>`, `content-type: application/json`, тело — JSON); между сервисами — **gRPC / gRPC-web** (так же, как административный API RPoem, `open_runo.admin.v1.Admin`). Потоковая передача — только когда она действительно нужна.
- **Существующие REST-эндпоинты переносятся по одному, когда до них доходят руки.** После переноса старый REST-маршрут удаляется. До завершения переноса они остаются, но **новые не добавляются**.
- **JSON унифицируется на RS-JSON**: чтение через `from_str_secure` / `parse_secure` (ограничения на размер, глубину и повторяющиеся ключи), запись через `rust_json::fast`. `prost` не добавляется в браузерный WASM (размер вырос бы).
- **Исключения (форматы, заданные чужим стандартом, а не наш собственный API)**: ACME, обратные вызовы OAuth/OIDC, вебхуки сторонних сервисов (например, GitHub), проверки работоспособности, раздача статических файлов, OTLP и т. п. Они принимают обычный HTTP, как требует стандарт.
- Эталонные реализации: `RPoem` (`open_runo.admin.v1.Admin`) и `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Примечание: соответствует ли код конкретного репозитория этой политике, зависит от самого репозитория. Этот раздел фиксирует политику и не означает, что код уже изменён.
