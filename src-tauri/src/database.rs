//! SQLiteデータベース層
//!
//! スキーマ方針(スナップショット型):
//! - `import_batches`: 取り込み1回 = 1行。「同一日×同一証券会社」でUNIQUE制約を張り、
//!   再取り込み時は既存バッチを削除(CASCADE)してから登録し直す = 上書き。
//! - `holdings`: 保有明細の積み上げ。バッチに従属。
//! - `stock_master`: 銘柄マスタ。SBIのCSVに含まれる33業種で自動更新され、
//!   構成比(セクター別)画面のセクター情報源になる。手動編集も想定。

use chrono::NaiveDate;
use rusqlite::{params, Connection};

use std::collections::HashMap;

use crate::models::{
    AssetHistoryPoint, Broker, CompositionItem, HoldingRecord, ImportBatchSummary, ImportSummary,
    MonthlyRealizedPnlPoint, ParsedRealizedPnlFile, ParsedSnapshot, ParsedTradeFile,
    RealizedPnlImportSummary, StockHistoryPoint, StockListItem, StockRealizedPnlItem,
    TradeAnalysis, TradeImportSummary, TradeListItem,
};

/// スキーマの初期化。アプリ起動時に毎回呼ぶ(冪等)。
pub fn initialize_database(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS import_batches (
            batch_id         INTEGER PRIMARY KEY AUTOINCREMENT,
            snapshot_date    TEXT NOT NULL,              -- 'YYYY-MM-DD'
            broker           TEXT NOT NULL,              -- 'sbi' | 'esmart'
            source_file_name TEXT,
            imported_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            UNIQUE (snapshot_date, broker)
        );

        CREATE TABLE IF NOT EXISTS holdings (
            holding_id                INTEGER PRIMARY KEY AUTOINCREMENT,
            batch_id                  INTEGER NOT NULL
                                      REFERENCES import_batches(batch_id) ON DELETE CASCADE,
            snapshot_date             TEXT NOT NULL,
            broker                    TEXT NOT NULL,
            account_type              TEXT NOT NULL,     -- '特定' | 'NISA成長投資枠' 等
            stock_code                TEXT NOT NULL,
            stock_name                TEXT NOT NULL,
            quantity                  REAL NOT NULL,
            average_acquisition_price REAL,
            current_price             REAL,
            market_value              REAL,
            acquisition_cost          REAL,
            profit_loss               REAL,
            profit_loss_rate          REAL
        );
        CREATE INDEX IF NOT EXISTS index_holdings_snapshot_date ON holdings (snapshot_date);
        CREATE INDEX IF NOT EXISTS index_holdings_stock_code    ON holdings (stock_code);

        CREATE TABLE IF NOT EXISTS stock_master (
            stock_code    TEXT PRIMARY KEY,
            stock_name    TEXT NOT NULL,
            sector_33     TEXT,   -- 東証33業種(SBIのCSVから自動更新 / 手動登録可)
            sector_detail TEXT
        );

        CREATE TABLE IF NOT EXISTS trade_records (
            trade_id             INTEGER PRIMARY KEY AUTOINCREMENT,
            broker               TEXT NOT NULL,     -- 'sbi' | 'esmart'
            trade_date           TEXT NOT NULL,     -- 約定日 'YYYY-MM-DD'
            settlement_date      TEXT,              -- 受渡日
            market               TEXT,
            stock_code           TEXT NOT NULL,
            stock_name           TEXT NOT NULL,
            side                 TEXT NOT NULL,     -- 'buy' | 'sell'
            quantity             REAL NOT NULL,
            price                REAL,
            settlement_amount    REAL,
            commission           REAL,
            account_type         TEXT NOT NULL,
            realized_profit_loss REAL,              -- 実現損益(e-smartの売りのみ。SBIはNULL)
            -- 重複取り込み防止用の決定的キー。行の内容+ファイル内出現番号から生成する。
            -- NULL列を含むUNIQUE制約はSQLiteでは重複を防げないため、文字列キーに畳み込む
            dedup_key            TEXT NOT NULL UNIQUE,
            imported_at          TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );
        CREATE INDEX IF NOT EXISTS index_trade_records_trade_date ON trade_records (trade_date);
        CREATE INDEX IF NOT EXISTS index_trade_records_stock_code ON trade_records (stock_code);

        -- SBIの譲渡益税明細CSV由来の確定実現損益。売却明細ごとに1行。
        -- 取引履歴(trade_records)とは別テーブルにし、実現損益はこちらを正とする。
        CREATE TABLE IF NOT EXISTS realized_pnl_records (
            realized_id          INTEGER PRIMARY KEY AUTOINCREMENT,
            broker               TEXT NOT NULL,
            trade_date           TEXT NOT NULL,     -- 約定日 'YYYY-MM-DD'
            settlement_date      TEXT,
            stock_code           TEXT NOT NULL,
            stock_name           TEXT NOT NULL,
            quantity             REAL NOT NULL,
            sell_amount          REAL,
            acquisition_date     TEXT,
            acquisition_amount   REAL,
            realized_profit_loss REAL NOT NULL,     -- 確定実現損益(符号付き)
            dedup_key            TEXT NOT NULL UNIQUE,
            imported_at          TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );
        CREATE INDEX IF NOT EXISTS index_realized_pnl_trade_date ON realized_pnl_records (trade_date);
        CREATE INDEX IF NOT EXISTS index_realized_pnl_stock_code ON realized_pnl_records (stock_code);
        "#,
    )
}

/// スナップショットをDBへ登録する。
/// 同一日・同一証券会社の既存バッチがあれば削除してから登録する(上書き動作)。
pub fn replace_snapshot(
    connection: &mut Connection,
    snapshot: &ParsedSnapshot,
    snapshot_date: NaiveDate,
    source_file_name: &str,
) -> rusqlite::Result<ImportSummary> {
    let snapshot_date_text = snapshot_date.format("%Y-%m-%d").to_string();
    let broker_identifier = snapshot.broker.identifier();

    let transaction = connection.transaction()?;

    let deleted_batch_count = transaction.execute(
        "DELETE FROM import_batches WHERE snapshot_date = ?1 AND broker = ?2",
        params![snapshot_date_text, broker_identifier],
    )?;

    transaction.execute(
        "INSERT INTO import_batches (snapshot_date, broker, source_file_name)
         VALUES (?1, ?2, ?3)",
        params![snapshot_date_text, broker_identifier, source_file_name],
    )?;
    let batch_id = transaction.last_insert_rowid();

    {
        let mut insert_holding_statement = transaction.prepare(
            "INSERT INTO holdings (
                batch_id, snapshot_date, broker, account_type,
                stock_code, stock_name, quantity,
                average_acquisition_price, current_price, market_value,
                acquisition_cost, profit_loss, profit_loss_rate
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )?;
        // 銘柄名は証券会社により略称のことがある(例: e-smartは「三菱商」、SBIは「三菱商事」)。
        // より長い名前の方が完全な名称である可能性が高いため、長い場合のみ上書きする
        let mut upsert_stock_master_statement = transaction.prepare(
            "INSERT INTO stock_master (stock_code, stock_name, sector_33, sector_detail)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (stock_code) DO UPDATE SET
                 stock_name    = CASE
                                     WHEN length(excluded.stock_name) > length(stock_master.stock_name)
                                     THEN excluded.stock_name
                                     ELSE stock_master.stock_name
                                 END,
                 sector_33     = COALESCE(excluded.sector_33,     stock_master.sector_33),
                 sector_detail = COALESCE(excluded.sector_detail, stock_master.sector_detail)",
        )?;

        for holding in &snapshot.holdings {
            insert_holding_statement.execute(params![
                batch_id,
                snapshot_date_text,
                broker_identifier,
                holding.account_type,
                holding.stock_code,
                holding.stock_name,
                holding.quantity,
                holding.average_acquisition_price,
                holding.current_price,
                holding.market_value,
                holding.acquisition_cost,
                holding.profit_loss,
                holding.profit_loss_rate,
            ])?;
            upsert_stock_master_statement.execute(params![
                holding.stock_code,
                holding.stock_name,
                holding.sector_33,
                holding.sector_detail,
            ])?;
        }
    }

    transaction.commit()?;

    Ok(ImportSummary {
        broker_display_name: snapshot.broker.display_name().to_owned(),
        snapshot_date: snapshot_date_text,
        imported_row_count: snapshot.holdings.len(),
        replaced_existing: deleted_batch_count > 0,
    })
}

/// 保有一覧: 証券会社ごとに最新スナップショットの明細を返す
pub fn fetch_latest_holdings(connection: &Connection) -> rusqlite::Result<Vec<HoldingRecord>> {
    let mut statement = connection.prepare(
        "SELECT
            h.broker, h.snapshot_date, h.account_type,
            h.stock_code, h.stock_name, h.quantity,
            h.average_acquisition_price, h.current_price, h.market_value,
            h.profit_loss, h.profit_loss_rate,
            m.sector_33
         FROM holdings AS h
         INNER JOIN (
             SELECT broker, MAX(snapshot_date) AS latest_snapshot_date
             FROM import_batches
             GROUP BY broker
         ) AS latest
             ON latest.broker = h.broker
            AND latest.latest_snapshot_date = h.snapshot_date
         LEFT JOIN stock_master AS m ON m.stock_code = h.stock_code
         ORDER BY h.stock_code, h.broker, h.account_type",
    )?;

    let holding_rows = statement.query_map([], |row| {
        let broker_identifier: String = row.get(0)?;
        let broker_display_name = Broker::from_identifier(&broker_identifier)
            .map(|broker| broker.display_name().to_owned())
            .unwrap_or_else(|| broker_identifier.clone());
        Ok(HoldingRecord {
            broker: broker_identifier,
            broker_display_name,
            snapshot_date: row.get(1)?,
            account_type: row.get(2)?,
            stock_code: row.get(3)?,
            stock_name: row.get(4)?,
            quantity: row.get(5)?,
            average_acquisition_price: row.get(6)?,
            current_price: row.get(7)?,
            market_value: row.get(8)?,
            profit_loss: row.get(9)?,
            profit_loss_rate: row.get(10)?,
            sector_33: row.get(11)?,
        })
    })?;

    holding_rows.collect()
}

/// 資産推移: 日付ごとの評価額合計・評価損益合計。
/// `broker_filter` に識別子('sbi' 等)を渡すとその証券会社のみ、Noneなら全社合算。
pub fn fetch_asset_history(
    connection: &Connection,
    broker_filter: Option<&str>,
) -> rusqlite::Result<Vec<AssetHistoryPoint>> {
    let mut statement = connection.prepare(
        "SELECT
            snapshot_date,
            COALESCE(SUM(market_value), 0.0),
            COALESCE(SUM(profit_loss), 0.0)
         FROM holdings
         WHERE (?1 IS NULL OR broker = ?1)
         GROUP BY snapshot_date
         ORDER BY snapshot_date",
    )?;

    let history_rows = statement.query_map(params![broker_filter], |row| {
        Ok(AssetHistoryPoint {
            snapshot_date: row.get(0)?,
            total_market_value: row.get(1)?,
            total_profit_loss: row.get(2)?,
        })
    })?;

    history_rows.collect()
}

/// 銘柄別分析用の銘柄リスト。
/// 全履歴からDISTINCTで取るため、売却済み(最新スナップショットに無い)銘柄も
/// 過去の推移を振り返れる。`is_currently_held` で保有中かどうかを区別する。
/// `broker_filter` に識別子を渡すと、その証券会社で保有履歴のある銘柄のみ返し、
/// 保有中判定もその証券会社の最新スナップショット内で評価する。
pub fn fetch_stock_list(
    connection: &Connection,
    broker_filter: Option<&str>,
) -> rusqlite::Result<Vec<StockListItem>> {
    let mut statement = connection.prepare(
        "SELECT
            m.stock_code,
            m.stock_name,
            m.sector_33,
            EXISTS (
                SELECT 1
                FROM holdings AS h
                INNER JOIN (
                    SELECT broker, MAX(snapshot_date) AS latest_snapshot_date
                    FROM import_batches
                    GROUP BY broker
                ) AS latest
                    ON latest.broker = h.broker
                   AND latest.latest_snapshot_date = h.snapshot_date
                WHERE h.stock_code = m.stock_code
                  AND (?1 IS NULL OR h.broker = ?1)
            ) AS is_currently_held
         FROM stock_master AS m
         WHERE EXISTS (
             SELECT 1 FROM holdings
             WHERE stock_code = m.stock_code
               AND (?1 IS NULL OR broker = ?1)
         )
         ORDER BY m.stock_code",
    )?;

    let stock_rows = statement.query_map(params![broker_filter], |row| {
        Ok(StockListItem {
            stock_code: row.get(0)?,
            stock_name: row.get(1)?,
            sector_33: row.get(2)?,
            is_currently_held: row.get(3)?,
        })
    })?;

    stock_rows.collect()
}

/// 銘柄別推移: 指定銘柄の日付ごとの数量・現在値・評価額・評価損益。
/// 複数の証券会社・口座区分で同一銘柄を持つ場合は合算する。
/// `broker_filter` に識別子を渡すとその証券会社のみ、Noneなら全社合算。
pub fn fetch_stock_history(
    connection: &Connection,
    stock_code: &str,
    broker_filter: Option<&str>,
) -> rusqlite::Result<Vec<StockHistoryPoint>> {
    let mut statement = connection.prepare(
        "SELECT
            snapshot_date,
            COALESCE(SUM(quantity), 0.0),
            MAX(current_price),
            COALESCE(SUM(market_value), 0.0),
            COALESCE(SUM(profit_loss), 0.0)
         FROM holdings
         WHERE stock_code = ?1
           AND (?2 IS NULL OR broker = ?2)
         GROUP BY snapshot_date
         ORDER BY snapshot_date",
    )?;

    let history_rows = statement.query_map(params![stock_code, broker_filter], |row| {
        Ok(StockHistoryPoint {
            snapshot_date: row.get(0)?,
            total_quantity: row.get(1)?,
            current_price: row.get(2)?,
            total_market_value: row.get(3)?,
            total_profit_loss: row.get(4)?,
        })
    })?;

    history_rows.collect()
}

/// 構成比: 証券会社ごとの最新スナップショットを対象に、銘柄単位で評価額を合算して返す。
/// セクターは `stock_master` から引く(SBIのCSV取り込みで自動更新される)。
/// セクター別の畳み込みはフロント側で行う。
/// `broker_filter` に識別子を渡すとその証券会社のみ、Noneなら全社合算。
pub fn fetch_composition(
    connection: &Connection,
    broker_filter: Option<&str>,
) -> rusqlite::Result<Vec<CompositionItem>> {
    let mut statement = connection.prepare(
        "SELECT
            h.stock_code,
            MAX(h.stock_name),
            MAX(m.sector_33),
            COALESCE(SUM(h.market_value), 0.0)
         FROM holdings AS h
         INNER JOIN (
             SELECT broker, MAX(snapshot_date) AS latest_snapshot_date
             FROM import_batches
             GROUP BY broker
         ) AS latest
             ON latest.broker = h.broker
            AND latest.latest_snapshot_date = h.snapshot_date
         LEFT JOIN stock_master AS m ON m.stock_code = h.stock_code
         WHERE (?1 IS NULL OR h.broker = ?1)
         GROUP BY h.stock_code
         ORDER BY SUM(h.market_value) DESC",
    )?;

    let composition_rows = statement.query_map(params![broker_filter], |row| {
        Ok(CompositionItem {
            stock_code: row.get(0)?,
            stock_name: row.get(1)?,
            sector_33: row.get(2)?,
            total_market_value: row.get(3)?,
        })
    })?;

    composition_rows.collect()
}

/// 取込履歴の一覧(スナップショット管理画面用)。新しい日付・証券会社順。
pub fn fetch_import_batches(connection: &Connection) -> rusqlite::Result<Vec<ImportBatchSummary>> {
    let mut statement = connection.prepare(
        "SELECT
            b.batch_id,
            b.broker,
            b.snapshot_date,
            b.source_file_name,
            b.imported_at,
            (SELECT COUNT(*) FROM holdings AS h WHERE h.batch_id = b.batch_id) AS holding_count
         FROM import_batches AS b
         ORDER BY b.snapshot_date DESC, b.broker",
    )?;

    let batch_rows = statement.query_map([], |row| {
        let broker_identifier: String = row.get(1)?;
        let broker_display_name = Broker::from_identifier(&broker_identifier)
            .map(|broker| broker.display_name().to_owned())
            .unwrap_or(broker_identifier);
        Ok(ImportBatchSummary {
            batch_id: row.get(0)?,
            broker_display_name,
            snapshot_date: row.get(2)?,
            source_file_name: row.get(3)?,
            imported_at: row.get(4)?,
            holding_count: row.get(5)?,
        })
    })?;

    batch_rows.collect()
}

/// 取込バッチを1件削除する。`holdings` は外部キーのCASCADEで自動的に削除される。
pub fn delete_import_batch(connection: &Connection, batch_id: i64) -> rusqlite::Result<usize> {
    connection.execute(
        "DELETE FROM import_batches WHERE batch_id = ?1",
        params![batch_id],
    )
}

// ---------------------------------------------------------------------------
// 取引履歴
// ---------------------------------------------------------------------------

/// 取引履歴をDBへ登録する(重複防止付き)。
///
/// 重複判定キーは行の内容(証券会社・約定日・銘柄・売買・数量・単価・口座区分・受渡金額)
/// にファイル内出現番号を加えた決定的な文字列。これにより:
/// - 毎日ダウンロードするCSVの期間重複 → 同じキーになり INSERT OR IGNORE でスキップ
/// - 正当に同一内容の取引が同日に複数ある場合 → 出現番号(0,1,...)で区別され両方保持
pub fn insert_trades(
    connection: &mut Connection,
    trade_file: &ParsedTradeFile,
) -> rusqlite::Result<TradeImportSummary> {
    let broker_identifier = trade_file.broker.identifier();
    let transaction = connection.transaction()?;

    let mut new_trade_count = 0usize;
    let mut duplicate_trade_count = 0usize;
    // 「内容キー → これまでの出現回数」。同一内容の行に 0,1,2... を割り当てる
    let mut occurrence_counter: HashMap<String, u32> = HashMap::new();

    {
        let mut insert_trade_statement = transaction.prepare(
            "INSERT OR IGNORE INTO trade_records (
                broker, trade_date, settlement_date, market,
                stock_code, stock_name, side, quantity, price,
                settlement_amount, commission, account_type,
                realized_profit_loss, dedup_key
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?;
        // 銘柄マスタの更新: 保有株側と同じく「より長い名前を優先」で名前を充実させる。
        // SBIの取引履歴は正式名(例: 木村化工機)、e-smartは略称(例: 木村化)のため、
        // 取引履歴からも正式名を取り込めるようにする(セクターは触らない)
        let mut upsert_stock_name_statement = transaction.prepare(
            "INSERT INTO stock_master (stock_code, stock_name) VALUES (?1, ?2)
             ON CONFLICT (stock_code) DO UPDATE SET
                 stock_name = CASE
                                  WHEN length(excluded.stock_name) > length(stock_master.stock_name)
                                  THEN excluded.stock_name
                                  ELSE stock_master.stock_name
                              END",
        )?;

        for trade in &trade_file.trades {
            let trade_date_text = trade.trade_date.format("%Y-%m-%d").to_string();
            let content_key = format!(
                "{}|{}|{}|{}|{}|{:?}|{:?}|{}",
                broker_identifier,
                trade_date_text,
                trade.stock_code,
                trade.side.identifier(),
                trade.quantity,
                trade.price,
                trade.settlement_amount,
                trade.account_type,
            );
            let occurrence_index = occurrence_counter
                .entry(content_key.clone())
                .and_modify(|count| *count += 1)
                .or_insert(0);
            let dedup_key = format!("{content_key}#{occurrence_index}");

            let inserted_row_count = insert_trade_statement.execute(params![
                broker_identifier,
                trade_date_text,
                trade
                    .settlement_date
                    .map(|date| date.format("%Y-%m-%d").to_string()),
                trade.market,
                trade.stock_code,
                trade.stock_name,
                trade.side.identifier(),
                trade.quantity,
                trade.price,
                trade.settlement_amount,
                trade.commission,
                trade.account_type,
                trade.realized_profit_loss,
                dedup_key,
            ])?;

            if inserted_row_count > 0 {
                new_trade_count += 1;
                upsert_stock_name_statement.execute(params![trade.stock_code, trade.stock_name])?;
            } else {
                duplicate_trade_count += 1;
            }
        }
    }

    transaction.commit()?;

    let mut trade_dates: Vec<chrono::NaiveDate> = trade_file
        .trades
        .iter()
        .map(|trade| trade.trade_date)
        .collect();
    trade_dates.sort();

    Ok(TradeImportSummary {
        broker_display_name: trade_file.broker.display_name().to_owned(),
        new_trade_count,
        duplicate_trade_count,
        earliest_trade_date: trade_dates
            .first()
            .map(|date| date.format("%Y-%m-%d").to_string()),
        latest_trade_date: trade_dates
            .last()
            .map(|date| date.format("%Y-%m-%d").to_string()),
    })
}

// ---------------------------------------------------------------------------
// 譲渡益税明細(確定実現損益)
// ---------------------------------------------------------------------------

/// 譲渡益税明細をDBへ登録する(重複防止付き)。
/// 重複キーは明細内容(証券会社・約定日・銘柄・数量・売却額・取得額・取得日・損益)に
/// ファイル内出現番号を加えた決定的な文字列。毎日の期間重複ダウンロードでも
/// 同一明細はスキップされる。
pub fn insert_realized_pnl(
    connection: &mut Connection,
    pnl_file: &ParsedRealizedPnlFile,
) -> rusqlite::Result<RealizedPnlImportSummary> {
    let broker_identifier = pnl_file.broker.identifier();
    let transaction = connection.transaction()?;

    let mut new_record_count = 0usize;
    let mut duplicate_record_count = 0usize;
    let mut total_realized_profit_loss = 0.0;
    let mut occurrence_counter: HashMap<String, u32> = HashMap::new();

    {
        let mut insert_statement = transaction.prepare(
            "INSERT OR IGNORE INTO realized_pnl_records (
                broker, trade_date, settlement_date, stock_code, stock_name,
                quantity, sell_amount, acquisition_date, acquisition_amount,
                realized_profit_loss, dedup_key
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        let mut upsert_stock_name_statement = transaction.prepare(
            "INSERT INTO stock_master (stock_code, stock_name) VALUES (?1, ?2)
             ON CONFLICT (stock_code) DO UPDATE SET
                 stock_name = CASE
                                  WHEN length(excluded.stock_name) > length(stock_master.stock_name)
                                  THEN excluded.stock_name
                                  ELSE stock_master.stock_name
                              END",
        )?;

        for record in &pnl_file.records {
            let trade_date_text = record.trade_date.format("%Y-%m-%d").to_string();
            let acquisition_date_text = record
                .acquisition_date
                .map(|date| date.format("%Y-%m-%d").to_string());
            let content_key = format!(
                "{}|{}|{}|{}|{:?}|{:?}|{:?}|{}",
                broker_identifier,
                trade_date_text,
                record.stock_code,
                record.quantity,
                record.sell_amount,
                record.acquisition_amount,
                acquisition_date_text,
                record.realized_profit_loss,
            );
            let occurrence_index = occurrence_counter
                .entry(content_key.clone())
                .and_modify(|count| *count += 1)
                .or_insert(0);
            let dedup_key = format!("{content_key}#{occurrence_index}");

            let inserted = insert_statement.execute(params![
                broker_identifier,
                trade_date_text,
                record
                    .settlement_date
                    .map(|date| date.format("%Y-%m-%d").to_string()),
                record.stock_code,
                record.stock_name,
                record.quantity,
                record.sell_amount,
                acquisition_date_text,
                record.acquisition_amount,
                record.realized_profit_loss,
                dedup_key,
            ])?;

            if inserted > 0 {
                new_record_count += 1;
                total_realized_profit_loss += record.realized_profit_loss;
                upsert_stock_name_statement
                    .execute(params![record.stock_code, record.stock_name])?;
            } else {
                duplicate_record_count += 1;
            }
        }
    }

    transaction.commit()?;

    let mut trade_dates: Vec<chrono::NaiveDate> = pnl_file
        .records
        .iter()
        .map(|record| record.trade_date)
        .collect();
    trade_dates.sort();

    Ok(RealizedPnlImportSummary {
        broker_display_name: pnl_file.broker.display_name().to_owned(),
        new_record_count,
        duplicate_record_count,
        total_realized_profit_loss,
        earliest_trade_date: trade_dates
            .first()
            .map(|date| date.format("%Y-%m-%d").to_string()),
        latest_trade_date: trade_dates
            .last()
            .map(|date| date.format("%Y-%m-%d").to_string()),
    })
}

/// 取引分析画面用の集計一式を返す。
///
/// 実現損益は「確定値」を正とする:
/// - e-smart: trade_records の売買損益列(realized_profit_loss)
/// - SBI: realized_pnl_records(譲渡益税明細CSV由来)
///   これらを (broker, trade_date, stock_code) 粒度で突き合わせ、確定損益を集計する。
///   譲渡益税明細が未取り込みのSBI売りは「実現損益不明」として集計から除外する
///   (概算はしない。正確性を優先する方針)。
///
/// 売買回数・手数料は従来どおり trade_records から数える。
/// `broker_filter`: 'sbi' 等。None=全社。
/// `start_date`('YYYY-MM-DD'): この日以降の約定のみ。None=全期間。
pub fn fetch_trade_analysis(
    connection: &Connection,
    broker_filter: Option<&str>,
    start_date: Option<&str>,
) -> rusqlite::Result<TradeAnalysis> {
    // 銘柄名はマスタの正式名を優先
    let mut master_name_statement =
        connection.prepare("SELECT stock_code, stock_name FROM stock_master")?;
    let master_names: HashMap<String, String> = master_name_statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let resolve_stock_name = |stock_code: &str, fallback_name: &str| -> String {
        master_names
            .get(stock_code)
            .cloned()
            .unwrap_or_else(|| fallback_name.to_owned())
    };

    // ---- 売買回数・手数料(trade_records) ----
    let (buy_trade_count, sell_trade_count, total_commission) = connection.query_row(
        "SELECT
            COALESCE(SUM(side = 'buy'), 0),
            COALESCE(SUM(side = 'sell'), 0),
            COALESCE(SUM(commission), 0.0)
         FROM trade_records
         WHERE (?1 IS NULL OR broker = ?1)
           AND (?2 IS NULL OR trade_date >= ?2)",
        params![broker_filter, start_date],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        },
    )?;

    // ---- 確定実現損益を1つのリストに統合 ----
    // (broker, trade_date, stock_code, stock_name, realized_pnl)
    struct ConfirmedPnl {
        #[allow(dead_code)]
        broker: String,
        trade_date: String,
        stock_code: String,
        stock_name: String,
        realized_profit_loss: f64,
    }
    let mut confirmed_list: Vec<ConfirmedPnl> = Vec::new();

    // e-smart: trade_records の売りで realized_profit_loss が入っているもの
    let mut esmart_statement = connection.prepare(
        "SELECT broker, trade_date, stock_code, stock_name, realized_profit_loss
         FROM trade_records
         WHERE side = 'sell' AND realized_profit_loss IS NOT NULL
           AND (?1 IS NULL OR broker = ?1)
           AND (?2 IS NULL OR trade_date >= ?2)",
    )?;
    let esmart_rows = esmart_statement.query_map(params![broker_filter, start_date], |row| {
        Ok(ConfirmedPnl {
            broker: row.get(0)?,
            trade_date: row.get(1)?,
            stock_code: row.get(2)?,
            stock_name: row.get(3)?,
            realized_profit_loss: row.get(4)?,
        })
    })?;
    for row in esmart_rows {
        confirmed_list.push(row?);
    }

    // SBI: realized_pnl_records(譲渡益税明細)
    let mut sbi_statement = connection.prepare(
        "SELECT broker, trade_date, stock_code, stock_name, realized_profit_loss
         FROM realized_pnl_records
         WHERE (?1 IS NULL OR broker = ?1)
           AND (?2 IS NULL OR trade_date >= ?2)",
    )?;
    let sbi_rows = sbi_statement.query_map(params![broker_filter, start_date], |row| {
        Ok(ConfirmedPnl {
            broker: row.get(0)?,
            trade_date: row.get(1)?,
            stock_code: row.get(2)?,
            stock_name: row.get(3)?,
            realized_profit_loss: row.get(4)?,
        })
    })?;
    for row in sbi_rows {
        confirmed_list.push(row?);
    }

    // SBIの売りのうち、譲渡益税明細で損益が判明していない件数を数える。
    // trade_records の SBI売り件数 - realized_pnl_records の該当件数(日・銘柄で突合)。
    // 簡易的に「realized_pnl_recordsに(broker,trade_date,stock_code)が存在しないSBI売り取引」を不明とする。
    let unknown_pnl_sell_count: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM trade_records AS t
         WHERE t.side = 'sell' AND t.broker = 'sbi'
           AND (?1 IS NULL OR t.broker = ?1)
           AND (?2 IS NULL OR t.trade_date >= ?2)
           AND NOT EXISTS (
               SELECT 1 FROM realized_pnl_records AS r
               WHERE r.broker = t.broker
                 AND r.trade_date = t.trade_date
                 AND r.stock_code = t.stock_code
           )",
        params![broker_filter, start_date],
        |row| row.get(0),
    )?;

    // ---- 集計 ----
    let mut total_realized_profit_loss = 0.0;
    let mut winning_sell_count = 0i64;
    let mut sell_count_with_known_pnl = 0i64;
    let mut monthly_totals: std::collections::BTreeMap<String, f64> =
        std::collections::BTreeMap::new();
    let mut stock_totals: HashMap<String, (f64, i64, String)> = HashMap::new();

    for confirmed in &confirmed_list {
        total_realized_profit_loss += confirmed.realized_profit_loss;
        sell_count_with_known_pnl += 1;
        if confirmed.realized_profit_loss > 0.0 {
            winning_sell_count += 1;
        }
        let month_key = confirmed.trade_date.chars().take(7).collect::<String>();
        *monthly_totals.entry(month_key).or_insert(0.0) += confirmed.realized_profit_loss;

        let stock_entry = stock_totals.entry(confirmed.stock_code.clone()).or_insert((
            0.0,
            0,
            confirmed.stock_name.clone(),
        ));
        stock_entry.0 += confirmed.realized_profit_loss;
        stock_entry.1 += 1;
    }

    let mut cumulative_total_profit_loss = 0.0;
    let monthly_points: Vec<MonthlyRealizedPnlPoint> = monthly_totals
        .into_iter()
        .map(|(month, confirmed)| {
            cumulative_total_profit_loss += confirmed;
            MonthlyRealizedPnlPoint {
                month,
                realized_profit_loss: confirmed,
                cumulative_total_profit_loss,
            }
        })
        .collect();

    let mut stock_items: Vec<StockRealizedPnlItem> = stock_totals
        .into_iter()
        .map(
            |(stock_code, (realized, sell_count, fallback_name))| StockRealizedPnlItem {
                stock_name: resolve_stock_name(&stock_code, &fallback_name),
                stock_code,
                realized_profit_loss: realized,
                sell_trade_count: sell_count,
            },
        )
        .collect();
    stock_items.sort_by(|left, right| {
        right
            .realized_profit_loss
            .partial_cmp(&left.realized_profit_loss)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ---- 直近の取引(trade_records、最大100件) ----
    let mut recent_statement = connection.prepare(
        "SELECT
            broker, trade_date, stock_code, stock_name, side,
            quantity, price, settlement_amount, account_type, realized_profit_loss
         FROM trade_records
         WHERE (?1 IS NULL OR broker = ?1)
           AND (?2 IS NULL OR trade_date >= ?2)
         ORDER BY trade_date DESC, trade_id DESC
         LIMIT 100",
    )?;
    // SBIの確定損益を (trade_date, stock_code) で引けるようにしておく
    // (同日同銘柄に複数明細がある場合は合算値を表示に使う)
    let mut sbi_pnl_lookup: HashMap<(String, String), f64> = HashMap::new();
    {
        let mut lookup_statement = connection.prepare(
            "SELECT trade_date, stock_code, SUM(realized_profit_loss)
             FROM realized_pnl_records
             WHERE broker = 'sbi'
             GROUP BY trade_date, stock_code",
        )?;
        let lookup_rows = lookup_statement.query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, f64>(2)?,
            ))
        })?;
        for row in lookup_rows {
            let (key, value) = row?;
            sbi_pnl_lookup.insert(key, value);
        }
    }
    let recent_trades: Vec<TradeListItem> = recent_statement
        .query_map(params![broker_filter, start_date], |row| {
            let broker_identifier: String = row.get(0)?;
            let trade_date: String = row.get(1)?;
            let stock_code: String = row.get(2)?;
            let fallback_name: String = row.get(3)?;
            let side: String = row.get(4)?;
            let broker_display_name = Broker::from_identifier(&broker_identifier)
                .map(|broker| broker.display_name().to_owned())
                .unwrap_or_else(|| broker_identifier.clone());
            let esmart_pnl: Option<f64> = row.get(9)?;
            // 表示用の確定損益: e-smartはtrade_recordsの値、SBIは譲渡益税明細から引く
            let confirmed_pnl = if side == "sell" {
                esmart_pnl.or_else(|| {
                    if broker_identifier == "sbi" {
                        sbi_pnl_lookup
                            .get(&(trade_date.clone(), stock_code.clone()))
                            .copied()
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            Ok(TradeListItem {
                broker_display_name,
                trade_date,
                stock_name: resolve_stock_name(&stock_code, &fallback_name),
                stock_code,
                side,
                quantity: row.get(5)?,
                price: row.get(6)?,
                settlement_amount: row.get(7)?,
                account_type: row.get(8)?,
                realized_profit_loss: confirmed_pnl,
            })
        })?
        .collect::<Result<_, _>>()?;

    Ok(TradeAnalysis {
        total_realized_profit_loss,
        total_commission,
        buy_trade_count,
        sell_trade_count,
        winning_sell_count,
        sell_count_with_known_pnl,
        unknown_pnl_sell_count,
        monthly_points,
        stock_items,
        recent_trades,
    })
}
