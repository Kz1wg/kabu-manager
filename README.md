# 保有株管理アプリ (kabu-manager)

SBI証券 / e-smart証券の保有株CSVをSQLiteに取り込み、記録・分析・可視化するデスクトップアプリ。
Tauri 2.x + Svelte 5 + Rust。ローカル完結(外部通信なし)。

## 現在の実装範囲(縦切りの最小構成)

```
CSVファイル選択 or ドラッグ&ドロップ
  → エンコーディング自動判定(UTF-8 BOM / UTF-8 / CP932)
  → 証券会社自動判別(ヘッダー内容から)
  → パース(カンマ区切り数値・"--"欠損・令和日付に対応)
  → SQLite登録(同一日×同一証券会社は上書き)
  → invoke → Svelteでソート可能なテーブル表示
```

加えて **資産推移タブ**(ECharts の2軸折れ線: 評価額・評価損益)を実装済み。
証券会社フィルタ(すべて / SBI証券 / e-smart証券 / 今後追加される証券会社)は
保有一覧・資産推移の両タブに連動する。保有一覧はフロント側で、資産推移は
SQL側(`WHERE broker = ?`)でフィルタしている。

## セットアップ

### 前提

- Rust(rustup推奨。`rustc 1.77` 以上)
- Node.js 20以上
- macOS: Xcode Command Line Tools / Windows: WebView2(通常プリインストール) と VS Build Tools



### 動作確認

1. アプリが起動したら、SBI証券の `sbi.csv` または e-smart証券の `Account.csv` を
   ウィンドウにドラッグ&ドロップ(またはボタンからファイル選択)
2. 「SBI証券 / 2026-07-06 / 13銘柄を取り込みました」のような表示が出て、
   保有一覧テーブルが更新される
3. 同じファイルをもう一度取り込むと「(同日データを上書き)」となり重複しない

## 設計メモ

### ファイル構成

```
src-tauri/src/
├── main.rs            エントリポイント(薄い)
├── lib.rs             Tauriビルダー、DB接続のセットアップ
├── commands.rs        #[tauri::command] 群。rusqliteはここより奥に閉じ込める
├── models.rs          共有データ型(Broker, ParsedHolding, HoldingRecord等)
├── broker_profile.rs  証券会社ごとの差異を吸収するプロファイル定義
├── csv_import.rs      デコード → 判別 → パース(保有株CSV)
├── trade_import.rs    取引履歴CSVのパース(TradeKabu / SaveFile取引版)
├── realized_pnl_import.rs  SBI譲渡益税明細CSVのパース(確定実現損益)
└── database.rs        スキーマ・登録・集計クエリ
src/
├── App.svelte             メイン画面(取り込みUI)
└── lib/HoldingsTable.svelte  ソート可能な保有一覧テーブル
```

### DBスキーマ(スナップショット型)

- `import_batches(batch_id, snapshot_date, broker, source_file_name, imported_at)`
  - `UNIQUE(snapshot_date, broker)` — 再取り込みは既存バッチをCASCADE削除して登録し直す
- `holdings(holding_id, batch_id→CASCADE, snapshot_date, broker, account_type,
   stock_code, stock_name, quantity, average_acquisition_price, current_price,
   market_value, acquisition_cost, profit_loss, profit_loss_rate)`
- `stock_master(stock_code, stock_name, sector_33, sector_detail)`
  - SBIのCSVに含まれる「33業種」「業種詳細」で自動更新。構成比画面のセクター情報源。
  - e-smart側の保有もコード経由でセクターを引ける(実データで確認済み)

DBファイルの場所(画面フッターにも表示):

- macOS: `~/Library/Application Support/com.kz1wg.kabu-manager/kabu_manager.db`
- Windows: `%APPDATA%\com.kz1wg.kabu-manager\kabu_manager.db`

### CSVフォーマットの観察結果(実サンプル準拠)

| | SBI証券(ポートフォリオ画面) | e-smart証券(残高照会) |
|---|---|---|
| エンコーディング | UTF-8(BOM付き) | CP932(Shift-JIS) |
| 日付 | CSV内に無し → 画面指定 or 今日 | タイトル行の「令和NN年MM月DD日」から抽出 |
| 特記事項 | 「銘柄」列が7列重複。非空セルの並びで[コード,銘柄名,市場]と判別 | タイトル行→ヘッダー→明細の構造 |
| セクター情報 | 33業種・業種詳細あり | なし |

※ briefでは「SBI=Shift-JIS・複数セクション」とあったが、これは口座管理画面のCSVの形式。
エンコーディングはバイト列から自動判定し、ヘッダー検出もセクションごとに再実行する設計に
してあるため、口座管理画面のCSVを追加対応する場合も `broker_profile.rs` と
`csv_import.rs` の列マッピング追加だけで済む見込み。

### 証券会社の追加方法

1. `models.rs` の `Broker` enum にバリアント追加
2. `broker_profile.rs` の `BROKER_PROFILES` に判定キーワードとヘッダー必須列を追加
3. `csv_import.rs` に列マッピング(パース関数)を追加

## 今後の候補

- EChartsのバンドルサイズ削減: `echarts/core` からの部分import に切り替え
- `stock_master` のセクター手動編集UI(現状はSBIのCSV取り込みで自動更新、
  またはDBを直接UPDATE)
- 実現損益(推定): 総平均法の性質を使い、スナップショット間の保有株数減少から
  実現損益を推定する機能。設計検討・型定義(`realized_pnl.rs`)まで着手済みだが
  未接続(lib.rsに未登録)。証券会社の取引履歴CSVが手に入れば、推定ではなく
  正確な実現損益を取り込む形に置き換えられる

## 実装済みタブ

- **保有一覧**: 最新スナップショットのソート可能テーブル(フロント側フィルタ)
- **資産推移**: 評価額・評価損益の2軸折れ線(SQL側フィルタ)
- **銘柄別分析**: 銘柄選択(保有中/売却済みグループ分け、証券会社フィルタに連動)
  → 評価額・評価損益の推移。複数証券会社の同一銘柄は合算
  (実データで 8058 = SBI 34株 + e-smart 14株 = 48株 を検証済み)。
  ツールチップに数量・現在値も表示
- **構成比**: 最新スナップショットの評価額構成。ツリーマップ(セクター→銘柄の階層、
  クリックでズームイン)と円グラフ(セクター別/銘柄別)を切り替え可能。
  セクターは `stock_master.sector_33` 由来で、未登録銘柄は「未分類」に集約
- **取引分析**: 取引履歴CSVと譲渡益税明細CSVから実現損益を可視化。
  KPI(実現損益・勝率・売買回数・手数料)、月別実現損益(棒)+累計(折れ線)、
  銘柄別実現損益ランキング、直近取引一覧。表示期間を選択可能
  (全期間 / 1ヶ月 / 3ヶ月 / 半年 / 1年。開始日はフロントで暦計算しRustに渡す)。
  取り込みは保有株CSVと同じドロップ/選択で、種類は内容から自動判別。
  実現損益は**確定値のみ**を扱う(概算はしない):
  - e-smart: 取引履歴CSV(TradeKabu)の売買損益列
  - SBI: 譲渡益税明細CSV(特定口座損益明細, SaveFile〜.csv)の損益金額列
  取引履歴・譲渡益税明細とも重複防止付き(行内容+ファイル内出現番号の
  決定的キーで `INSERT OR IGNORE`)。毎日の期間重複ダウンロードは自動スキップ。
  SBIの売りで譲渡益税明細が未取込のものは「損益不明」として集計から除外
  (実データ335明細で損益合計を独立検算し一致を確認済み)。
  ※CSVは3種類とも自動判別: 譲渡益税明細(「損益金額/徴収額」列)>
  取引履歴(「約定日」列)> 保有株、の順に判定。
  ※NELなど非LF改行の混在に対応(譲渡益税明細CSVで実際に発生)。
- **取込履歴管理**: 取込済みスナップショットの一覧(証券会社・日付・銘柄数・
  元ファイル名・取込日時)と削除。削除は確認ダイアログ必須・取り消し不可。
  `import_batches` を削除すると外部キーのCASCADEで対応する `holdings` も
  自動的に削除される(実データで動作確認済み)。※取引履歴(trade_records)は
  この画面の対象外
