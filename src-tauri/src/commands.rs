//! フロントエンド(Svelte)へ公開する Tauri コマンド群
//!
//! 方針: rusqlite はこの層より奥(database.rs)に閉じ込め、
//! フロントには serde でシリアライズ済みの集計データのみを渡す。

use std::path::Path;
use std::sync::Mutex;

use chrono::NaiveDate;
use tauri::State;

use crate::csv_import;
use crate::database;
use crate::models::{
    AssetHistoryPoint, CompositionItem, CsvImportOutcome, HoldingRecord, ImportBatchSummary,
    ImportSummary, StockHistoryPoint, StockListItem, TradeAnalysis,
};
use crate::realized_pnl_import;
use crate::trade_import;

/// アプリ全体で共有する状態(SQLite接続)
pub struct AppState {
    pub database_connection: Mutex<rusqlite::Connection>,
    pub database_file_path: String,
}

/// 保有株CSVを取り込む。
/// - 証券会社・エンコーディングはファイル内容から自動判別
/// - スナップショット日付の優先順位: 画面での指定 > CSV内の日付 > 今日
/// - 同一日・同一証券会社は上書き
#[tauri::command]
pub fn import_holdings_csv(
    state: State<AppState>,
    file_path: String,
    snapshot_date_override: Option<String>,
) -> Result<ImportSummary, String> {
    let raw_bytes = std::fs::read(&file_path)
        .map_err(|error| format!("ファイルを読み込めませんでした: {error}"))?;

    let snapshot = csv_import::parse_holdings_csv(&raw_bytes).map_err(|error| error.to_string())?;

    let override_date = match snapshot_date_override.as_deref().map(str::trim) {
        Some(date_text) if !date_text.is_empty() => Some(
            NaiveDate::parse_from_str(date_text, "%Y-%m-%d")
                .map_err(|_| format!("日付の形式が不正です(YYYY-MM-DD): {date_text}"))?,
        ),
        _ => None,
    };
    let snapshot_date = override_date
        .or(snapshot.snapshot_date)
        .unwrap_or_else(|| chrono::Local::now().date_naive());

    let source_file_name = Path::new(&file_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_path.clone());

    let mut connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;

    database::replace_snapshot(&mut connection, &snapshot, snapshot_date, &source_file_name)
        .map_err(|error| format!("データベース登録に失敗しました: {error}"))
}

/// 保有一覧(証券会社ごとの最新スナップショット)を取得する
#[tauri::command]
pub fn fetch_latest_holdings(state: State<AppState>) -> Result<Vec<HoldingRecord>, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_latest_holdings(&connection).map_err(|error| error.to_string())
}

/// 資産推移(日付ごとの評価額・損益の合計)を取得する。
/// `broker_filter`: 'sbi' 等の識別子。省略/nullなら全社合算。
#[tauri::command]
pub fn fetch_asset_history(
    state: State<AppState>,
    broker_filter: Option<String>,
) -> Result<Vec<AssetHistoryPoint>, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_asset_history(&connection, broker_filter.as_deref())
        .map_err(|error| error.to_string())
}

/// DBファイルの保存場所を返す(画面フッターに表示する)
#[tauri::command]
pub fn fetch_database_path(state: State<AppState>) -> String {
    state.database_file_path.clone()
}

/// 銘柄別分析用の銘柄リスト(売却済み銘柄も含む)を取得する。
/// `broker_filter`: 'sbi' 等の識別子。省略/nullなら全社。
#[tauri::command]
pub fn fetch_stock_list(
    state: State<AppState>,
    broker_filter: Option<String>,
) -> Result<Vec<StockListItem>, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_stock_list(&connection, broker_filter.as_deref())
        .map_err(|error| error.to_string())
}

/// 指定銘柄の推移(数量・現在値・評価額・評価損益)を取得する。
/// `broker_filter`: 'sbi' 等の識別子。省略/nullなら全社合算。
#[tauri::command]
pub fn fetch_stock_history(
    state: State<AppState>,
    stock_code: String,
    broker_filter: Option<String>,
) -> Result<Vec<StockHistoryPoint>, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_stock_history(&connection, &stock_code, broker_filter.as_deref())
        .map_err(|error| error.to_string())
}

/// 構成比(最新スナップショットの銘柄別評価額+セクター)を取得する。
/// `broker_filter`: 'sbi' 等の識別子。省略/nullなら全社合算。
#[tauri::command]
pub fn fetch_composition(
    state: State<AppState>,
    broker_filter: Option<String>,
) -> Result<Vec<CompositionItem>, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_composition(&connection, broker_filter.as_deref())
        .map_err(|error| error.to_string())
}

/// 取込履歴の一覧(スナップショット管理画面用)を取得する
#[tauri::command]
pub fn fetch_import_batches(state: State<AppState>) -> Result<Vec<ImportBatchSummary>, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_import_batches(&connection).map_err(|error| error.to_string())
}

/// 取込バッチを1件削除する(誤取り込みの取り消し用)。取り消し不可。
#[tauri::command]
pub fn delete_import_batch(state: State<AppState>, batch_id: i64) -> Result<(), String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    let deleted_row_count = database::delete_import_batch(&connection, batch_id)
        .map_err(|error| format!("削除に失敗しました: {error}"))?;
    if deleted_row_count == 0 {
        return Err("指定された取込履歴が見つかりませんでした".to_owned());
    }
    Ok(())
}

/// CSVの種類(保有株 or 取引履歴)を内容から自動判別して取り込む。
/// 取引履歴CSV(約定日列を含む)なら重複防止付きで取引を登録し、
/// それ以外は従来どおり保有株スナップショットとして登録する。
#[tauri::command]
pub fn import_csv_auto(
    state: State<AppState>,
    file_path: String,
    snapshot_date_override: Option<String>,
) -> Result<CsvImportOutcome, String> {
    let raw_bytes = std::fs::read(&file_path)
        .map_err(|error| format!("ファイルを読み込めませんでした: {error}"))?;
    let csv_text = crate::csv_import::decode_csv_bytes(&raw_bytes);

    // 譲渡益税明細CSVは取引履歴CSVより先に判定する(どちらも約定日列を含むため)
    if realized_pnl_import::looks_like_realized_pnl_csv(&csv_text) {
        let pnl_file = realized_pnl_import::parse_realized_pnl_csv(&raw_bytes)
            .map_err(|error| error.to_string())?;
        let mut connection = state
            .database_connection
            .lock()
            .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
        let summary = database::insert_realized_pnl(&mut connection, &pnl_file)
            .map_err(|error| format!("データベース登録に失敗しました: {error}"))?;
        return Ok(CsvImportOutcome::RealizedPnl(summary));
    }

    if trade_import::looks_like_trade_csv(&csv_text) {
        let trade_file =
            trade_import::parse_trade_csv(&raw_bytes).map_err(|error| error.to_string())?;
        let mut connection = state
            .database_connection
            .lock()
            .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
        let summary = database::insert_trades(&mut connection, &trade_file)
            .map_err(|error| format!("データベース登録に失敗しました: {error}"))?;
        return Ok(CsvImportOutcome::Trade(summary));
    }

    import_holdings_csv(state, file_path, snapshot_date_override)
        .map(CsvImportOutcome::Holdings)
}

/// 取引分析(実現損益サマリー・月別推移・銘柄別・直近取引)を取得する。
/// `broker_filter`: 'sbi' 等の識別子。省略/nullなら全社。
/// 取引分析(実現損益サマリー・期間別推移・銘柄別・直近取引)を取得する。
/// `broker_filter`: 'sbi' 等の識別子。省略/nullなら全社。
/// `start_date`: 'YYYY-MM-DD'。省略/nullなら全期間。
/// `granularity`: 'day' | 'week' | 'month'。グラフの集計粒度。
#[tauri::command]
pub fn fetch_trade_analysis(
    state: State<AppState>,
    broker_filter: Option<String>,
    start_date: Option<String>,
    granularity: String,
) -> Result<TradeAnalysis, String> {
    let connection = state
        .database_connection
        .lock()
        .map_err(|_| "データベース接続のロックに失敗しました".to_owned())?;
    database::fetch_trade_analysis(
        &connection,
        broker_filter.as_deref(),
        start_date.as_deref(),
        database::TradeAnalysisGranularity::from_key(&granularity),
    )
    .map_err(|error| error.to_string())
}
