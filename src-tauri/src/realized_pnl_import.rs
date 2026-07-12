//! SBI証券 譲渡益税明細(特定口座損益明細)CSVの取り込み
//!
//! 実ファイル(SaveFile_*.csv)の観察結果:
//! - CP932。改行に NEL(0x85) が混在するため正規化してから処理する。
//! - 冒頭に「特定口座損益明細」タイトルと期間・合計のメタデータセクション。
//! - 明細ヘッダー:
//!   `銘柄コード,銘柄,譲渡益取消区分,約定日,数量,取引,受渡日,売却/決済金額,費用,
//!    取得/新規年月日,取得/新規金額,損益金額/徴収額,地方税`
//! - 明細行: 銘柄コードは空白パディング(`6136     `)、数量は `12株`、
//!   取引は `現物売`、損益は符号付き(`+60`/`-308`)、欠損は `--`。
//!   各行で「売却金額 - 取得金額 = 損益金額列」が成立する(実データで検証済み)。
//! - 受渡日ごとに `譲渡益税徴収額,,,...` の集計行が挟まる(取引列が空なので除外される)。

use chrono::NaiveDate;

use crate::csv_import::{build_header_index_map, field_text, record_has_columns, CsvImportError};
use crate::models::{Broker, ParsedRealizedPnl, ParsedRealizedPnlFile};

/// CSV本文が譲渡益税明細CSVかどうか(他CSVとの振り分けに使う)
pub fn looks_like_realized_pnl_csv(csv_text: &str) -> bool {
    // 「損益金額/徴収額」列と「特定口座損益明細」タイトルは他CSVには無い
    csv_text.contains("損益金額/徴収額")
        && (csv_text.contains("特定口座損益明細") || csv_text.contains("譲渡益税徴収額合計"))
}

/// NEL(0x85)等の非LF改行をLFに正規化してからデコードする。
/// (decode_csv_bytesはBOM/UTF-8/CP932は処理するが改行コードは触らないため)
fn decode_and_normalize_newlines(raw_bytes: &[u8]) -> String {
    let decoded = crate::csv_import::decode_csv_bytes(raw_bytes);
    decoded.replace(['\u{0085}', '\u{2028}', '\u{2029}'], "\n")
}

const REALIZED_PNL_HEADER_COLUMNS: &[&str] = &["銘柄コード", "約定日", "取引", "損益金額/徴収額"];

/// `12株` / `1,234` / `+60` / `-308` / `--` を数値化する
fn parse_realized_number(raw_text: &str) -> Option<f64> {
    let cleaned: String = raw_text
        .trim()
        .chars()
        .filter(|character| !matches!(character, ',' | '+' | '株' | '円' | ' ' | '"'))
        .collect();
    if cleaned.is_empty() || cleaned == "--" || cleaned == "-" {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

/// `2026/01/05` / `2026年01月05日` 形式の日付をパースする
fn parse_flexible_date(raw_text: &str) -> Option<NaiveDate> {
    let trimmed = raw_text.trim().trim_matches('"').trim();
    NaiveDate::parse_from_str(trimmed, "%Y/%m/%d")
        .or_else(|_| NaiveDate::parse_from_str(trimmed, "%Y年%m月%d日"))
        .ok()
}

/// 譲渡益税明細CSVのバイト列をパースするエントリポイント
pub fn parse_realized_pnl_csv(raw_bytes: &[u8]) -> Result<ParsedRealizedPnlFile, CsvImportError> {
    let csv_text = decode_and_normalize_newlines(raw_bytes);
    if !looks_like_realized_pnl_csv(&csv_text) {
        return Err(CsvImportError::UnknownBroker);
    }

    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(csv_text.as_bytes());

    let mut all_records: Vec<csv::StringRecord> = Vec::new();
    for record_result in csv_reader.records() {
        all_records.push(record_result?);
    }

    let mut records: Vec<ParsedRealizedPnl> = Vec::new();
    let mut header_index_map = None;

    for record in &all_records {
        if record_has_columns(record, REALIZED_PNL_HEADER_COLUMNS) {
            header_index_map = Some(build_header_index_map(record));
            continue;
        }
        let Some(current_header_map) = header_index_map.as_ref() else {
            continue;
        };

        // 「取引」列が現物売でない行(譲渡益税徴収額の集計行など)は除外する
        let is_sell = field_text(record, current_header_map, "取引")
            .map(|text| text.contains('売'))
            .unwrap_or(false);
        if !is_sell {
            continue;
        }

        let Some(stock_code) = field_text(record, current_header_map, "銘柄コード")
            .map(|code| code.trim_matches('"').trim().to_owned())
            .filter(|code| !code.is_empty())
        else {
            continue;
        };
        let Some(trade_date) =
            field_text(record, current_header_map, "約定日").and_then(parse_flexible_date)
        else {
            continue;
        };
        let Some(quantity) =
            field_text(record, current_header_map, "数量").and_then(parse_realized_number)
        else {
            continue;
        };
        // 損益金額/徴収額列。符号付き。ここが取れない行はスキップ
        let Some(realized_profit_loss) = field_text(record, current_header_map, "損益金額/徴収額")
            .and_then(|text| {
                let trimmed = text.trim();
                if trimmed == "--" || trimmed.is_empty() {
                    Some(0.0)
                } else {
                    parse_signed_number(trimmed)
                }
            })
        else {
            continue;
        };

        records.push(ParsedRealizedPnl {
            trade_date,
            settlement_date: field_text(record, current_header_map, "受渡日")
                .and_then(parse_flexible_date),
            stock_code,
            stock_name: field_text(record, current_header_map, "銘柄")
                .map(|name| name.trim_matches('"').trim().to_owned())
                .unwrap_or_default(),
            quantity,
            sell_amount: field_text(record, current_header_map, "売却/決済金額")
                .and_then(parse_realized_number),
            acquisition_date: field_text(record, current_header_map, "取得/新規年月日")
                .and_then(parse_flexible_date),
            acquisition_amount: field_text(record, current_header_map, "取得/新規金額")
                .and_then(parse_realized_number),
            realized_profit_loss,
        });
    }

    if header_index_map.is_none() {
        return Err(CsvImportError::HeaderNotFound);
    }
    if records.is_empty() {
        return Err(CsvImportError::NoDataRows);
    }

    Ok(ParsedRealizedPnlFile {
        broker: Broker::Sbi,
        records,
    })
}

/// 符号付き数値(`+60` / `-308` / `1,234`)をパースする
fn parse_signed_number(raw_text: &str) -> Option<f64> {
    let cleaned: String = raw_text
        .trim()
        .chars()
        .filter(|character| !matches!(character, ',' | '+' | ' ' | '"'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok()
}
