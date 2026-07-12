//! 取引履歴CSVの取り込み処理
//!
//! 実ファイルの観察結果:
//! - e-smart証券(TradeKabu.csv): CP932。ヘッダーは
//!   `約定日,受渡日,市場,銘柄名,銘柄コード,売買区分,数量,単価,受渡金額,ポイント利用,手数料,口座区分,売買損益`
//!   売買区分は 買/売。売り行の「売買損益」に実現損益が入る(買いは0)。
//!   日付は `YYYY/MM/DD`。
//! - SBI証券(SaveFile〜.csv): 既知のフォーマット
//!   `約定日,銘柄,銘柄コード,市場,取引,期限,預り,課税,約定数量,約定単価,手数料/諸経費等,税額,受渡日,受渡金額/決済損益`
//!   に基づくヘッダー名駆動の実装。※実サンプル未検証。
//!   「取引」列は 株式現物買/株式現物売 等。実現損益の列は無いためNoneとする
//!   (SBIの実現損益は別途「譲渡益税明細」CSVが必要)。

use chrono::NaiveDate;

use crate::csv_import::{
    build_header_index_map, decode_csv_bytes, field_text, normalize_account_type, numeric_field,
    record_has_columns, CsvImportError,
};
use crate::models::{Broker, ParsedTrade, ParsedTradeFile, TradeSide};

/// CSV本文が取引履歴CSVかどうか(保有株CSVとの振り分けに使う)
pub fn looks_like_trade_csv(csv_text: &str) -> bool {
    csv_text.contains("約定日")
}

/// 取引履歴CSVの証券会社を判別する。
/// 「売買損益」列はe-smart固有、「約定数量」「預り」はSBI固有。
fn detect_trade_broker(csv_text: &str) -> Option<Broker> {
    if csv_text.contains("売買区分") && csv_text.contains("売買損益") {
        return Some(Broker::Esmart);
    }
    if csv_text.contains("約定数量") && csv_text.contains("預り") {
        return Some(Broker::Sbi);
    }
    None
}

/// `2026/07/10` / `2026-07-10` 形式の日付をパースする
fn parse_trade_date(raw_text: &str) -> Option<NaiveDate> {
    let trimmed = raw_text.trim();
    NaiveDate::parse_from_str(trimmed, "%Y/%m/%d")
        .or_else(|_| NaiveDate::parse_from_str(trimmed, "%Y-%m-%d"))
        .ok()
}

/// 取引履歴CSVのバイト列をパースするエントリポイント
pub fn parse_trade_csv(raw_bytes: &[u8]) -> Result<ParsedTradeFile, CsvImportError> {
    let csv_text = decode_csv_bytes(raw_bytes);
    let broker = detect_trade_broker(&csv_text).ok_or(CsvImportError::UnknownBroker)?;

    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(csv_text.as_bytes());

    let mut all_records: Vec<csv::StringRecord> = Vec::new();
    for record_result in csv_reader.records() {
        all_records.push(record_result?);
    }

    match broker {
        Broker::Esmart => parse_esmart_trades(&all_records),
        Broker::Sbi => parse_sbi_trades(&all_records),
    }
}

// ---------------------------------------------------------------------------
// e-smart証券 (TradeKabu.csv)
// ---------------------------------------------------------------------------

const ESMART_TRADE_HEADER_COLUMNS: &[&str] = &["約定日", "銘柄コード", "売買区分", "数量"];

fn parse_esmart_trades(
    records: &[csv::StringRecord],
) -> Result<ParsedTradeFile, CsvImportError> {
    let mut trades: Vec<ParsedTrade> = Vec::new();
    let mut header_index_map = None;

    for record in records {
        if record_has_columns(record, ESMART_TRADE_HEADER_COLUMNS) {
            header_index_map = Some(build_header_index_map(record));
            continue;
        }
        let Some(current_header_map) = header_index_map.as_ref() else {
            continue;
        };

        let Some(trade_date) =
            field_text(record, current_header_map, "約定日").and_then(parse_trade_date)
        else {
            continue;
        };
        let Some(stock_code) = field_text(record, current_header_map, "銘柄コード")
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(side) =
            field_text(record, current_header_map, "売買区分").and_then(|text| match text {
                text if text.contains('買') => Some(TradeSide::Buy),
                text if text.contains('売') => Some(TradeSide::Sell),
                _ => None,
            })
        else {
            continue;
        };
        let Some(quantity) = numeric_field(record, current_header_map, "数量") else {
            continue;
        };

        // 売買損益は売り行のみ意味を持つ(買い行は常に0が入っている)
        let realized_profit_loss = match side {
            TradeSide::Sell => numeric_field(record, current_header_map, "売買損益"),
            TradeSide::Buy => None,
        };

        trades.push(ParsedTrade {
            trade_date,
            settlement_date: field_text(record, current_header_map, "受渡日")
                .and_then(parse_trade_date),
            market: field_text(record, current_header_map, "市場")
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            stock_code,
            stock_name: field_text(record, current_header_map, "銘柄名")
                .unwrap_or_default()
                .to_owned(),
            side,
            quantity,
            price: numeric_field(record, current_header_map, "単価"),
            settlement_amount: numeric_field(record, current_header_map, "受渡金額"),
            commission: numeric_field(record, current_header_map, "手数料"),
            account_type: field_text(record, current_header_map, "口座区分")
                .map(normalize_account_type)
                .unwrap_or_default(),
            realized_profit_loss,
        });
    }

    if header_index_map.is_none() {
        return Err(CsvImportError::HeaderNotFound);
    }
    if trades.is_empty() {
        return Err(CsvImportError::NoDataRows);
    }

    Ok(ParsedTradeFile {
        broker: Broker::Esmart,
        trades,
    })
}

// ---------------------------------------------------------------------------
// SBI証券 (SaveFile〜.csv) ※実サンプル未検証
// ---------------------------------------------------------------------------

const SBI_TRADE_HEADER_COLUMNS: &[&str] = &["約定日", "銘柄コード", "取引", "約定数量"];

fn parse_sbi_trades(records: &[csv::StringRecord]) -> Result<ParsedTradeFile, CsvImportError> {
    let mut trades: Vec<ParsedTrade> = Vec::new();
    let mut header_index_map = None;

    for record in records {
        if record_has_columns(record, SBI_TRADE_HEADER_COLUMNS) {
            header_index_map = Some(build_header_index_map(record));
            continue;
        }
        let Some(current_header_map) = header_index_map.as_ref() else {
            continue;
        };

        let Some(trade_date) =
            field_text(record, current_header_map, "約定日").and_then(parse_trade_date)
        else {
            continue;
        };
        let Some(stock_code) = field_text(record, current_header_map, "銘柄コード")
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        // 「取引」列: 株式現物買 / 株式現物売 / NISA預り など取引種別が入る
        let Some(side) =
            field_text(record, current_header_map, "取引").and_then(|text| match text {
                text if text.contains('買') => Some(TradeSide::Buy),
                text if text.contains('売') => Some(TradeSide::Sell),
                _ => None,
            })
        else {
            continue;
        };
        let Some(quantity) = numeric_field(record, current_header_map, "約定数量") else {
            continue;
        };

        trades.push(ParsedTrade {
            trade_date,
            settlement_date: field_text(record, current_header_map, "受渡日")
                .and_then(parse_trade_date),
            market: field_text(record, current_header_map, "市場")
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            stock_code,
            stock_name: field_text(record, current_header_map, "銘柄")
                .unwrap_or_default()
                .to_owned(),
            side,
            quantity,
            price: numeric_field(record, current_header_map, "約定単価"),
            settlement_amount: numeric_field(record, current_header_map, "受渡金額/決済損益"),
            commission: numeric_field(record, current_header_map, "手数料/諸経費等"),
            account_type: field_text(record, current_header_map, "預り")
                .map(normalize_account_type)
                .unwrap_or_default(),
            // SBIの取引履歴CSVには実現損益の列が無い(譲渡益税明細CSVが別途必要)
            realized_profit_loss: None,
        });
    }

    if header_index_map.is_none() {
        return Err(CsvImportError::HeaderNotFound);
    }
    if trades.is_empty() {
        return Err(CsvImportError::NoDataRows);
    }

    Ok(ParsedTradeFile {
        broker: Broker::Sbi,
        trades,
    })
}
