//! CSV取り込み処理
//!
//! 流れ: バイト列 → エンコーディング自動判定でデコード → 証券会社判別
//!       → ヘッダー行検出 → 行パース → `ParsedSnapshot`
//!
//! 実ファイルの観察結果:
//! - SBI証券(ポートフォリオ画面): UTF-8 BOM付き。「銘柄」列が7つ重複しており、
//!   コード・銘柄名は重複列のうち非空セルの並び順で判別する。
//!   数値は `"1,234"` のようにカンマ区切りで引用符付き。欠損は `--`。
//! - e-smart証券(残高照会): CP932。先頭に
//!   `[ 残高照会 ]- 国内株式 （...） -令和08年07月06日(月) ...` というタイトル行があり、
//!   ここからスナップショット日付を抽出できる。

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::broker_profile::{detect_broker, BrokerProfile};
use crate::models::{Broker, ParsedHolding, ParsedSnapshot};

/// 取り込み処理で発生しうるエラー
#[derive(Debug, thiserror::Error)]
pub enum CsvImportError {
    #[error("CSVの内容からどの証券会社か判別できませんでした")]
    UnknownBroker,
    #[error("ヘッダー行が見つかりませんでした")]
    HeaderNotFound,
    #[error("有効な保有明細行が1件もありませんでした")]
    NoDataRows,
    #[error("CSVの読み取りに失敗しました: {0}")]
    CsvRead(#[from] csv::Error),
}

/// バイト列を文字列にデコードする。
/// UTF-8 BOM → UTF-8 → Shift-JIS(CP932) の順で試す。
pub fn decode_csv_bytes(raw_bytes: &[u8]) -> String {
    const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    if raw_bytes.starts_with(UTF8_BOM) {
        return String::from_utf8_lossy(&raw_bytes[UTF8_BOM.len()..]).into_owned();
    }
    if let Ok(valid_utf8_text) = std::str::from_utf8(raw_bytes) {
        return valid_utf8_text.to_owned();
    }
    // encoding_rs の SHIFT_JIS は実質 windows-31j(CP932) 互換
    let (decoded_text, _encoding_used, _had_errors) = encoding_rs::SHIFT_JIS.decode(raw_bytes);
    decoded_text.into_owned()
}

/// `"1,234"` / `+2.85%` / `--` などの表記ゆれを吸収して数値化する
pub fn parse_number(raw_text: &str) -> Option<f64> {
    let cleaned_text: String = raw_text
        .trim()
        .chars()
        .filter(|character| !matches!(character, ',' | '%' | '+' | '円' | '株'))
        .collect();
    if cleaned_text.is_empty() || cleaned_text == "--" || cleaned_text == "-" {
        return None;
    }
    cleaned_text.parse::<f64>().ok()
}

/// タイトル行などから日付を抽出する。
/// 「令和NN年MM月DD日」と「YYYY/MM/DD」「YYYY年MM月DD日」に対応。
pub fn extract_date_from_text(text: &str) -> Option<NaiveDate> {
    // 令和元年 = 2019年
    const REIWA_BASE_YEAR: i32 = 2018;
    let era_pattern = regex::Regex::new(r"令和(\d{1,2})年(\d{1,2})月(\d{1,2})日").ok()?;
    if let Some(captures) = era_pattern.captures(text) {
        let reiwa_year: i32 = captures[1].parse().ok()?;
        let month: u32 = captures[2].parse().ok()?;
        let day: u32 = captures[3].parse().ok()?;
        return NaiveDate::from_ymd_opt(REIWA_BASE_YEAR + reiwa_year, month, day);
    }
    let western_pattern = regex::Regex::new(r"(20\d{2})[/年](\d{1,2})[/月](\d{1,2})日?").ok()?;
    if let Some(captures) = western_pattern.captures(text) {
        let year: i32 = captures[1].parse().ok()?;
        let month: u32 = captures[2].parse().ok()?;
        let day: u32 = captures[3].parse().ok()?;
        return NaiveDate::from_ymd_opt(year, month, day);
    }
    None
}

/// 口座区分の表記を正規化する(証券会社間で揃える)
pub fn normalize_account_type(raw_account_type: &str) -> String {
    let trimmed = raw_account_type.trim();
    if trimmed.contains("つみたて投資枠") {
        return "NISAつみたて投資枠".to_owned();
    }
    if trimmed.contains("成長投資枠") {
        return "NISA成長投資枠".to_owned();
    }
    if trimmed.contains("特定") {
        return "特定".to_owned();
    }
    if trimmed.contains("一般") && trimmed.contains("NISA") {
        return "一般NISA".to_owned();
    }
    trimmed.to_owned()
}

/// CSVファイルのバイト列をパースしてスナップショットを得るエントリポイント
pub fn parse_holdings_csv(raw_bytes: &[u8]) -> Result<ParsedSnapshot, CsvImportError> {
    let csv_text = decode_csv_bytes(raw_bytes);
    let profile = detect_broker(&csv_text).ok_or(CsvImportError::UnknownBroker)?;

    // 引用符内にカンマを含むため、行分割は csv クレートに任せる。
    // 行ごとに列数が違う(タイトル行・セクション見出し)ので flexible を有効にする。
    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(csv_text.as_bytes());

    let mut all_records: Vec<csv::StringRecord> = Vec::new();
    for record_result in csv_reader.records() {
        all_records.push(record_result?);
    }

    match profile.broker {
        Broker::Sbi => parse_sbi_records(profile, &all_records),
        Broker::Esmart => parse_esmart_records(profile, &all_records, &csv_text),
    }
}

/// レコードが指定した列名をすべて含むか(ヘッダー行判定の汎用版)
pub fn record_has_columns(record: &csv::StringRecord, required_columns: &[&str]) -> bool {
    required_columns
        .iter()
        .all(|column_name| record.iter().any(|cell| cell.trim() == *column_name))
}

/// レコードがヘッダー行かどうかをプロファイルの必須列名で判定する
fn is_header_record(record: &csv::StringRecord, profile: &BrokerProfile) -> bool {
    record_has_columns(record, profile.header_required_columns)
}

/// ヘッダー行から「列名 → 列番号のリスト」を作る(SBIは同名列が複数あるためVecで持つ)
pub fn build_header_index_map(header_record: &csv::StringRecord) -> HashMap<String, Vec<usize>> {
    let mut header_index_map: HashMap<String, Vec<usize>> = HashMap::new();
    for (column_index, column_name) in header_record.iter().enumerate() {
        header_index_map
            .entry(column_name.trim().to_owned())
            .or_default()
            .push(column_index);
    }
    header_index_map
}

/// 指定した列名の最初の列からセル値を取り出す
pub fn field_text<'a>(
    record: &'a csv::StringRecord,
    header_index_map: &HashMap<String, Vec<usize>>,
    column_name: &str,
) -> Option<&'a str> {
    let column_index = *header_index_map.get(column_name)?.first()?;
    record.get(column_index).map(str::trim)
}

/// 同名列が複数ある場合も含め、最初に数値としてパースできたセルを返す。
/// (SBIの「現在値」は2列あり、2列目は↑↓の矢印記号のため)
pub fn numeric_field(
    record: &csv::StringRecord,
    header_index_map: &HashMap<String, Vec<usize>>,
    column_name: &str,
) -> Option<f64> {
    let column_indexes = header_index_map.get(column_name)?;
    column_indexes
        .iter()
        .filter_map(|column_index| record.get(*column_index))
        .find_map(parse_number)
}

// ---------------------------------------------------------------------------
// SBI証券
// ---------------------------------------------------------------------------

/// SBI証券ポートフォリオCSVのパース。
/// 「銘柄」列が7列重複しているため、そのグループ内の非空セルを
/// [銘柄コード, 銘柄名, 市場] の順とみなして取り出す。
fn parse_sbi_records(
    profile: &BrokerProfile,
    records: &[csv::StringRecord],
) -> Result<ParsedSnapshot, CsvImportError> {
    let mut holdings: Vec<ParsedHolding> = Vec::new();
    let mut header_index_map: Option<HashMap<String, Vec<usize>>> = None;

    for record in records {
        if is_header_record(record, profile) {
            // セクションが複数ある形式(口座管理画面CSV等)でも、
            // セクションごとにヘッダーを検出し直すことで対応する
            header_index_map = Some(build_header_index_map(record));
            continue;
        }
        let Some(current_header_map) = header_index_map.as_ref() else {
            continue; // ヘッダー行より前(タイトル行など)は読み飛ばす
        };

        let Some((stock_code, stock_name)) = extract_sbi_code_and_name(record, current_header_map)
        else {
            continue;
        };
        let Some(quantity) = numeric_field(record, current_header_map, "保有株数") else {
            continue;
        };

        let account_type = field_text(record, current_header_map, "預り区分")
            .map(normalize_account_type)
            .unwrap_or_default();

        holdings.push(ParsedHolding {
            account_type,
            stock_code,
            stock_name,
            quantity,
            average_acquisition_price: numeric_field(record, current_header_map, "取得単価"),
            current_price: numeric_field(record, current_header_map, "現在値"),
            market_value: numeric_field(record, current_header_map, "評価額"),
            acquisition_cost: numeric_field(record, current_header_map, "買付金額"),
            profit_loss: numeric_field(record, current_header_map, "評価損益"),
            profit_loss_rate: numeric_field(record, current_header_map, "評価損益(%)"),
            sector_33: field_text(record, current_header_map, "33業種")
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            sector_detail: field_text(record, current_header_map, "業種詳細")
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
        });
    }

    if header_index_map.is_none() {
        return Err(CsvImportError::HeaderNotFound);
    }
    if holdings.is_empty() {
        return Err(CsvImportError::NoDataRows);
    }

    Ok(ParsedSnapshot {
        broker: Broker::Sbi,
        // SBIのポートフォリオCSVには日付が含まれないため、呼び出し側で補完する
        snapshot_date: None,
        holdings,
    })
}

/// 「銘柄」重複列グループから銘柄コードと銘柄名を取り出す
fn extract_sbi_code_and_name(
    record: &csv::StringRecord,
    header_index_map: &HashMap<String, Vec<usize>>,
) -> Option<(String, String)> {
    let stock_column_indexes = header_index_map.get("銘柄")?;
    let non_empty_values: Vec<&str> = stock_column_indexes
        .iter()
        .filter_map(|column_index| record.get(*column_index))
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .collect();

    // 非空セルの並び: [銘柄コード, 銘柄名, 市場] を期待
    let stock_code = (*non_empty_values.first()?).to_owned();
    let stock_name = (*non_empty_values.get(1)?).to_owned();

    // 銘柄コードは英数字4〜5桁(例: 8058, 262A)のみ許可し、集計行等を除外する
    let looks_like_stock_code = (4..=5).contains(&stock_code.len())
        && stock_code.chars().all(|character| character.is_ascii_alphanumeric());
    if !looks_like_stock_code {
        return None;
    }
    Some((stock_code, stock_name))
}

// ---------------------------------------------------------------------------
// e-smart証券
// ---------------------------------------------------------------------------

/// e-smart証券 残高照会CSVのパース。
/// タイトル行(令和日付を含む)→ヘッダー行→明細行の構造。
/// 複数セクションが縦に並ぶ場合もヘッダーを検出し直して継続する。
fn parse_esmart_records(
    profile: &BrokerProfile,
    records: &[csv::StringRecord],
    full_csv_text: &str,
) -> Result<ParsedSnapshot, CsvImportError> {
    let snapshot_date = extract_date_from_text(full_csv_text);

    let mut holdings: Vec<ParsedHolding> = Vec::new();
    let mut header_index_map: Option<HashMap<String, Vec<usize>>> = None;

    for record in records {
        if is_header_record(record, profile) {
            header_index_map = Some(build_header_index_map(record));
            continue;
        }
        let Some(current_header_map) = header_index_map.as_ref() else {
            continue;
        };

        let Some(stock_code) = field_text(record, current_header_map, "銘柄コード")
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(stock_name) = field_text(record, current_header_map, "銘柄名")
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(quantity) = numeric_field(record, current_header_map, "数量") else {
            continue;
        };

        let account_type = field_text(record, current_header_map, "区分")
            .map(normalize_account_type)
            .unwrap_or_default();

        holdings.push(ParsedHolding {
            account_type,
            stock_code,
            stock_name,
            quantity,
            average_acquisition_price: numeric_field(record, current_header_map, "買付単価"),
            current_price: numeric_field(record, current_header_map, "現在値"),
            market_value: numeric_field(record, current_header_map, "時価評価額"),
            acquisition_cost: numeric_field(record, current_header_map, "買付金額"),
            profit_loss: numeric_field(record, current_header_map, "評価損益(参考)"),
            profit_loss_rate: numeric_field(record, current_header_map, "評価損益率"),
            sector_33: None,
            sector_detail: None,
        });
    }

    if header_index_map.is_none() {
        return Err(CsvImportError::HeaderNotFound);
    }
    if holdings.is_empty() {
        return Err(CsvImportError::NoDataRows);
    }

    Ok(ParsedSnapshot {
        broker: Broker::Esmart,
        snapshot_date,
        holdings,
    })
}
