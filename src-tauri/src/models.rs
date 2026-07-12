//! アプリ全体で共有するデータ型定義

use serde::Serialize;

/// 対応証券会社の識別子
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Broker {
    Sbi,
    Esmart,
}

impl Broker {
    /// DBに保存する安定した識別子(表示名は変わりうるのでASCIIで固定)
    pub fn identifier(&self) -> &'static str {
        match self {
            Broker::Sbi => "sbi",
            Broker::Esmart => "esmart",
        }
    }

    /// 画面表示用の名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Broker::Sbi => "SBI証券",
            Broker::Esmart => "e-smart証券",
        }
    }

    pub fn from_identifier(identifier: &str) -> Option<Broker> {
        match identifier {
            "sbi" => Some(Broker::Sbi),
            "esmart" => Some(Broker::Esmart),
            _ => None,
        }
    }
}

/// CSVから読み取った1銘柄分の保有明細(DB登録前の中間表現)
#[derive(Debug, Clone)]
pub struct ParsedHolding {
    pub account_type: String,
    pub stock_code: String,
    pub stock_name: String,
    pub quantity: f64,
    pub average_acquisition_price: Option<f64>,
    pub current_price: Option<f64>,
    pub market_value: Option<f64>,
    pub acquisition_cost: Option<f64>,
    pub profit_loss: Option<f64>,
    pub profit_loss_rate: Option<f64>,
    /// SBIのCSVにのみ含まれる33業種区分(銘柄マスタ更新に使う)
    pub sector_33: Option<String>,
    pub sector_detail: Option<String>,
}

/// CSVファイル1本分のパース結果
#[derive(Debug)]
pub struct ParsedSnapshot {
    pub broker: Broker,
    /// CSV内から日付を特定できた場合のみ Some (e-smartはタイトル行に令和日付がある)
    pub snapshot_date: Option<chrono::NaiveDate>,
    pub holdings: Vec<ParsedHolding>,
}

/// 取り込み完了時にフロントへ返すサマリー
#[derive(Debug, Serialize)]
pub struct ImportSummary {
    pub broker_display_name: String,
    pub snapshot_date: String,
    pub imported_row_count: usize,
    /// 同一日・同一証券会社の既存データを上書きした場合 true
    pub replaced_existing: bool,
}

/// 保有一覧画面に渡す1行分のレコード
#[derive(Debug, Serialize)]
pub struct HoldingRecord {
    /// 証券会社の安定識別子('sbi' 等)。フィルタ値として使う
    pub broker: String,
    pub broker_display_name: String,
    pub snapshot_date: String,
    pub account_type: String,
    pub stock_code: String,
    pub stock_name: String,
    pub quantity: f64,
    pub average_acquisition_price: Option<f64>,
    pub current_price: Option<f64>,
    pub market_value: Option<f64>,
    pub profit_loss: Option<f64>,
    pub profit_loss_rate: Option<f64>,
    pub sector_33: Option<String>,
}

/// 資産推移グラフ用の1日分の集計値
#[derive(Debug, Serialize)]
pub struct AssetHistoryPoint {
    pub snapshot_date: String,
    pub total_market_value: f64,
    pub total_profit_loss: f64,
}

/// 銘柄別分析のドロップダウンに出す1銘柄分の情報
#[derive(Debug, Serialize)]
pub struct StockListItem {
    pub stock_code: String,
    pub stock_name: String,
    pub sector_33: Option<String>,
    /// 最新スナップショットに存在するか(false = 売却済み等)
    pub is_currently_held: bool,
}

/// 銘柄別推移グラフ用の1日分の集計値(証券会社をまたいで合算)
#[derive(Debug, Serialize)]
pub struct StockHistoryPoint {
    pub snapshot_date: String,
    pub total_quantity: f64,
    /// その日の現在値(証券会社間で同一のはずだが安全のためMAXを採る)
    pub current_price: Option<f64>,
    pub total_market_value: f64,
    pub total_profit_loss: f64,
}

/// スナップショット管理画面に渡す1回分の取り込み履歴
#[derive(Debug, Serialize)]
pub struct ImportBatchSummary {
    pub batch_id: i64,
    pub broker_display_name: String,
    pub snapshot_date: String,
    pub source_file_name: Option<String>,
    pub imported_at: String,
    pub holding_count: i64,
}

/// 構成比グラフ用の1銘柄分の集計値(最新スナップショット・証券会社合算)
#[derive(Debug, Serialize)]
pub struct CompositionItem {
    pub stock_code: String,
    pub stock_name: String,
    /// 未登録の場合はフロント側で「未分類」として扱う
    pub sector_33: Option<String>,
    pub total_market_value: f64,
}

// ---------------------------------------------------------------------------
// 取引履歴
// ---------------------------------------------------------------------------

/// 売買の別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}

impl TradeSide {
    pub fn identifier(&self) -> &'static str {
        match self {
            TradeSide::Buy => "buy",
            TradeSide::Sell => "sell",
        }
    }
}

/// CSVから読み取った1件分の取引(DB登録前の中間表現)
#[derive(Debug, Clone)]
pub struct ParsedTrade {
    pub trade_date: chrono::NaiveDate,
    pub settlement_date: Option<chrono::NaiveDate>,
    pub market: Option<String>,
    pub stock_code: String,
    pub stock_name: String,
    pub side: TradeSide,
    pub quantity: f64,
    pub price: Option<f64>,
    pub settlement_amount: Option<f64>,
    pub commission: Option<f64>,
    pub account_type: String,
    /// 実現損益。e-smartは売り行の「売買損益」列から取得。
    /// SBIの取引履歴CSVには含まれないためNone(買いも常にNone)
    pub realized_profit_loss: Option<f64>,
}

/// 取引履歴CSV1本分のパース結果
#[derive(Debug)]
pub struct ParsedTradeFile {
    pub broker: Broker,
    pub trades: Vec<ParsedTrade>,
}

/// 取引履歴の取り込み結果サマリー
#[derive(Debug, Serialize)]
pub struct TradeImportSummary {
    pub broker_display_name: String,
    pub new_trade_count: usize,
    /// 既に登録済みでスキップした件数(毎日の重複取り込み対策)
    pub duplicate_trade_count: usize,
    pub earliest_trade_date: Option<String>,
    pub latest_trade_date: Option<String>,
}

/// CSV自動判別取り込みの結果(保有株CSV or 取引履歴CSV)
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CsvImportOutcome {
    Holdings(ImportSummary),
    Trade(TradeImportSummary),
    RealizedPnl(RealizedPnlImportSummary),
}

/// 月別実現損益(グラフ用)
#[derive(Debug, Serialize)]
pub struct MonthlyRealizedPnlPoint {
    /// 'YYYY-MM'
    pub month: String,
    /// 確定実現損益(e-smart=売買損益列 / SBI=譲渡益税明細)
    pub realized_profit_loss: f64,
    pub cumulative_total_profit_loss: f64,
}

/// 銘柄別実現損益(ランキング用)
#[derive(Debug, Serialize)]
pub struct StockRealizedPnlItem {
    pub stock_code: String,
    pub stock_name: String,
    pub realized_profit_loss: f64,
    pub sell_trade_count: i64,
}

/// 取引一覧の1行
#[derive(Debug, Serialize)]
pub struct TradeListItem {
    pub broker_display_name: String,
    pub trade_date: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 'buy' | 'sell'
    pub side: String,
    pub quantity: f64,
    pub price: Option<f64>,
    pub settlement_amount: Option<f64>,
    pub account_type: String,
    /// 確定実現損益(e-smart=売買損益列 / SBI=譲渡益税明細で突合できた場合)
    pub realized_profit_loss: Option<f64>,
}

/// 取引分析画面に渡す集計一式(1回のinvokeで全部返す)
#[derive(Debug, Serialize)]
pub struct TradeAnalysis {
    /// 確定実現損益の合計(e-smart売買損益 + SBI譲渡益税明細)
    pub total_realized_profit_loss: f64,
    pub total_commission: f64,
    pub buy_trade_count: i64,
    pub sell_trade_count: i64,
    /// 実現損益が判明している売りのうち損益プラスの件数
    pub winning_sell_count: i64,
    /// 実現損益が判明している売りの総数(勝率の分母)
    pub sell_count_with_known_pnl: i64,
    /// SBIの売りで譲渡益税明細が未取り込みのため損益不明の件数
    pub unknown_pnl_sell_count: i64,
    pub monthly_points: Vec<MonthlyRealizedPnlPoint>,
    pub stock_items: Vec<StockRealizedPnlItem>,
    pub recent_trades: Vec<TradeListItem>,
}


// ---------------------------------------------------------------------------
// 譲渡益税明細(SBIの確定実現損益)
// ---------------------------------------------------------------------------

/// 譲渡益税明細CSVから読み取った1件分の売却損益(DB登録前の中間表現)
#[derive(Debug, Clone)]
pub struct ParsedRealizedPnl {
    pub trade_date: chrono::NaiveDate,
    pub settlement_date: Option<chrono::NaiveDate>,
    pub stock_code: String,
    pub stock_name: String,
    pub quantity: f64,
    pub sell_amount: Option<f64>,
    pub acquisition_date: Option<chrono::NaiveDate>,
    pub acquisition_amount: Option<f64>,
    pub realized_profit_loss: f64,
}

/// 譲渡益税明細CSV1本分のパース結果
#[derive(Debug)]
pub struct ParsedRealizedPnlFile {
    pub broker: Broker,
    pub records: Vec<ParsedRealizedPnl>,
}

/// 譲渡益税明細の取り込み結果サマリー
#[derive(Debug, Serialize)]
pub struct RealizedPnlImportSummary {
    pub broker_display_name: String,
    pub new_record_count: usize,
    pub duplicate_record_count: usize,
    pub total_realized_profit_loss: f64,
    pub earliest_trade_date: Option<String>,
    pub latest_trade_date: Option<String>,
}
