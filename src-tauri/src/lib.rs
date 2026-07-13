//! 保有株管理アプリ - Tauri バックエンド

mod broker_profile;
mod commands;
mod csv_import;
mod database;
mod models;
mod realized_pnl_import;
mod trade_import;

use std::sync::Mutex;

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // DBファイルは OS 標準のアプリデータディレクトリに置く。
            //   macOS:   ~/Library/Application Support/com.kz1wg.kabu-manager/kabu_manager.db
            //   Windows: %APPDATA%\com.kz1wg.kabu-manager\kabu_manager.db
            // 場所は画面フッターにも表示する(fetch_database_path)。
            let app_data_directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_directory)?;
            let database_file_path = app_data_directory.join("kabu_manager.db");

            let connection = rusqlite::Connection::open(&database_file_path)?;
            database::initialize_database(&connection)?;

            app.manage(AppState {
                database_connection: Mutex::new(connection),
                database_file_path: database_file_path.to_string_lossy().into_owned(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::import_holdings_csv,
            commands::fetch_latest_holdings,
            commands::fetch_asset_history,
            commands::fetch_stock_list,
            commands::fetch_stock_history,
            commands::fetch_composition,
            commands::fetch_import_batches,
            commands::delete_import_batch,
            commands::import_csv_auto,
            commands::fetch_trade_analysis,
            commands::fetch_database_path,
        ])
        .run(tauri::generate_context!())
        .expect("Tauriアプリの起動に失敗しました");
}
