//! 証券会社ごとのCSVフォーマット差異を吸収する設定
//!
//! 新しい証券会社を追加するときは、
//! 1. models.rs の `Broker` にバリアントを追加
//! 2. ここに `BrokerProfile` を1つ追加
//! 3. csv_import.rs に列マッピング(パース関数)を追加
//!    の3ステップで対応する。

use crate::models::Broker;

/// 証券会社ごとのCSVレイアウト設定
pub struct BrokerProfile {
    pub broker: Broker,
    /// このキーワードがすべてCSV本文に含まれていればこの証券会社と判定する
    pub detection_keywords: &'static [&'static str],
    /// ヘッダー行と判定するために必要な列名(すべて含む行をヘッダーとみなす)
    pub header_required_columns: &'static [&'static str],
}

/// 対応している全証券会社のプロファイル。判定は先頭から順に試す。
pub const BROKER_PROFILES: &[BrokerProfile] = &[
    // SBI証券: ポートフォリオ画面のCSV。
    // UTF-8(BOM付き)で出力されるが、口座管理画面のCSVはShift-JISのため
    // エンコーディングはプロファイルではなくバイト列から自動判定する。
    BrokerProfile {
        broker: Broker::Sbi,
        detection_keywords: &["預り区分", "保有株数"],
        header_required_columns: &["預り区分", "保有株数", "取得単価"],
    },
    // e-smart証券: 残高照会のCSV。CP932(Shift-JIS)。
    // 先頭に「[ 残高照会 ]- ... 令和NN年MM月DD日」というタイトル行がある。
    BrokerProfile {
        broker: Broker::Esmart,
        detection_keywords: &["銘柄名", "銘柄コード", "買付単価"],
        header_required_columns: &["銘柄名", "銘柄コード", "数量"],
    },
];

/// CSV本文の内容から証券会社を自動判別する
pub fn detect_broker(csv_text: &str) -> Option<&'static BrokerProfile> {
    BROKER_PROFILES.iter().find(|profile| {
        profile
            .detection_keywords
            .iter()
            .all(|keyword| csv_text.contains(keyword))
    })
}
