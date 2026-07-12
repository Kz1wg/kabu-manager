<script>
  /**
   * メイン画面
   * - CSV取り込み(ファイル選択 / ドラッグ&ドロップ)は全タブ共通
   * - タブ: 保有一覧 / 資産推移
   * - 証券会社フィルタは両タブに連動する
   *   (保有一覧はフロント側でフィルタ、資産推移はSQL側でフィルタ)
   */
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
  import HoldingsTable from "./lib/HoldingsTable.svelte";
  import AssetHistoryChart from "./lib/AssetHistoryChart.svelte";
  import StockAnalysisChart from "./lib/StockAnalysisChart.svelte";
  import CompositionChart from "./lib/CompositionChart.svelte";
  import ImportBatchManager from "./lib/ImportBatchManager.svelte";
  import TradeAnalysisPanel from "./lib/TradeAnalysisPanel.svelte";

  const tabDefinitions = [
    { key: "holdings", label: "保有一覧" },
    { key: "history", label: "資産推移" },
    { key: "stock", label: "銘柄別分析" },
    { key: "composition", label: "構成比" },
    { key: "trades", label: "取引分析" },
    { key: "manage", label: "取込履歴管理" },
  ];
  let currentTab = $state("holdings");

  let holdings = $state([]);
  let databasePath = $state("");
  let statusMessages = $state([]);
  let snapshotDateOverride = $state(""); // 空なら「CSV内の日付 > 今日」の順で自動決定
  let isDraggingOver = $state(false);
  let isImporting = $state(false);

  /** 表示対象の証券会社識別子('sbi' 等)。"all" なら全社 */
  let selectedBrokerFilter = $state("all");

  /** データに含まれる証券会社の一覧(識別子と表示名のペア)。増えるとボタンも自動で増える */
  const availableBrokers = $derived.by(() => {
    const brokerNameByIdentifier = new Map();
    for (const record of holdings) {
      brokerNameByIdentifier.set(record.broker, record.broker_display_name);
    }
    return [...brokerNameByIdentifier.entries()]
      .map(([identifier, displayName]) => ({ identifier, displayName }))
      .sort((left, right) =>
        left.displayName.localeCompare(right.displayName, "ja")
      );
  });

  /** フィルタ適用後の保有一覧(テーブルと合計はこれを使う) */
  const visibleHoldings = $derived(
    selectedBrokerFilter === "all"
      ? holdings
      : holdings.filter((record) => record.broker === selectedBrokerFilter)
  );

  function countHoldingsForBroker(brokerIdentifier) {
    return holdings.filter((record) => record.broker === brokerIdentifier)
      .length;
  }

  function pushStatusMessage(text, isError = false) {
    statusMessages = [...statusMessages, { text, isError, at: new Date() }].slice(-5);
  }

  async function reloadHoldings() {
    holdings = await invoke("fetch_latest_holdings");
  }

  async function importCsvFile(filePath) {
    isImporting = true;
    try {
      // 保有株CSVか取引履歴CSVかはRust側で内容から自動判別する
      const outcome = await invoke("import_csv_auto", {
        filePath,
        snapshotDateOverride: snapshotDateOverride || null,
      });
      if (outcome.kind === "holdings") {
        const overwriteNote = outcome.replaced_existing ? "(同日データを上書き)" : "";
        pushStatusMessage(
          `保有株: ${outcome.broker_display_name} / ${outcome.snapshot_date} / ` +
            `${outcome.imported_row_count}銘柄を取り込みました ${overwriteNote}`
        );
        await reloadHoldings();
      } else if (outcome.kind === "trade") {
        const dateRangeText =
          outcome.earliest_trade_date && outcome.latest_trade_date
            ? `${outcome.earliest_trade_date}〜${outcome.latest_trade_date}`
            : "";
        pushStatusMessage(
          `取引履歴: ${outcome.broker_display_name} ${dateRangeText} / ` +
            `新規${outcome.new_trade_count}件を取り込みました` +
            (outcome.duplicate_trade_count > 0
              ? `(取込済み${outcome.duplicate_trade_count}件はスキップ)`
              : "")
        );
      } else if (outcome.kind === "realized_pnl") {
        const dateRangeText =
          outcome.earliest_trade_date && outcome.latest_trade_date
            ? `${outcome.earliest_trade_date}〜${outcome.latest_trade_date}`
            : "";
        const sign = outcome.total_realized_profit_loss > 0 ? "+" : "";
        pushStatusMessage(
          `譲渡益税明細: ${outcome.broker_display_name} ${dateRangeText} / ` +
            `新規${outcome.new_record_count}件(実現損益 ${sign}${Math.round(outcome.total_realized_profit_loss).toLocaleString("ja-JP")}円)を取り込みました` +
            (outcome.duplicate_record_count > 0
              ? `(取込済み${outcome.duplicate_record_count}件はスキップ)`
              : "")
        );
      }
    } catch (errorMessage) {
      pushStatusMessage(`取り込み失敗: ${errorMessage}`, true);
    } finally {
      isImporting = false;
    }
  }

  async function selectAndImportCsvFiles() {
    const selectedPaths = await openFileDialog({
      multiple: true,
      filters: [{ name: "CSVファイル", extensions: ["csv"] }],
    });
    if (!selectedPaths) return;
    for (const filePath of selectedPaths) {
      await importCsvFile(filePath);
    }
  }

  $effect(() => {
    let unlistenDragDrop;

    (async () => {
      databasePath = await invoke("fetch_database_path");
      await reloadHoldings();

      // ウィンドウへのファイルドロップを受け付ける
      unlistenDragDrop = await getCurrentWebview().onDragDropEvent(async (event) => {
        if (event.payload.type === "over") {
          isDraggingOver = true;
        } else if (event.payload.type === "drop") {
          isDraggingOver = false;
          for (const droppedPath of event.payload.paths) {
            if (droppedPath.toLowerCase().endsWith(".csv")) {
              await importCsvFile(droppedPath);
            } else {
              pushStatusMessage(`CSVファイルではありません: ${droppedPath}`, true);
            }
          }
        } else {
          isDraggingOver = false;
        }
      });
    })();

    return () => {
      if (unlistenDragDrop) unlistenDragDrop();
    };
  });
</script>

<main class:dragging={isDraggingOver}>
  <header>
    <h1>保有株管理</h1>
    <div class="import-controls">
      <label class="date-override">
        取込日付(任意):
        <input
          type="date"
          bind:value={snapshotDateOverride}
          title="空欄なら CSV内の日付 → 今日 の順で自動決定します"
        />
      </label>
      <button onclick={selectAndImportCsvFiles} disabled={isImporting}>
        {isImporting ? "取り込み中..." : "CSVを選択して取り込み"}
      </button>
    </div>
  </header>

  <p class="drop-hint">
    保有株CSV・取引履歴CSV・SBI譲渡益税明細CSVをここにドラッグ&ドロップしても
    取り込めます(種類・証券会社・文字コードは自動判別。取引履歴・譲渡益税明細は
    重複取り込みを自動でスキップ)
  </p>

  {#if statusMessages.length > 0}
    <ul class="status-list">
      {#each statusMessages as message}
        <li class:error={message.isError}>{message.text}</li>
      {/each}
    </ul>
  {/if}

  <nav class="tab-bar">
    {#each tabDefinitions as tab (tab.key)}
      <button
        class="tab-button"
        class:active={currentTab === tab.key}
        onclick={() => (currentTab = tab.key)}
      >
        {tab.label}
      </button>
    {/each}
  </nav>

  {#if availableBrokers.length > 0}
    <div class="broker-filter" role="group" aria-label="証券会社で絞り込み">
      <button
        class="filter-button"
        class:active={selectedBrokerFilter === "all"}
        onclick={() => (selectedBrokerFilter = "all")}
      >
        すべて ({holdings.length})
      </button>
      {#each availableBrokers as broker (broker.identifier)}
        <button
          class="filter-button"
          class:active={selectedBrokerFilter === broker.identifier}
          onclick={() => (selectedBrokerFilter = broker.identifier)}
        >
          {broker.displayName} ({countHoldingsForBroker(broker.identifier)})
        </button>
      {/each}
    </div>
  {/if}

  {#if currentTab === "holdings"}
    <HoldingsTable holdings={visibleHoldings} />
  {:else if currentTab === "history"}
    <AssetHistoryChart brokerFilter={selectedBrokerFilter} />
  {:else if currentTab === "stock"}
    <StockAnalysisChart brokerFilter={selectedBrokerFilter} />
  {:else if currentTab === "composition"}
    <CompositionChart brokerFilter={selectedBrokerFilter} />
  {:else if currentTab === "trades"}
    <TradeAnalysisPanel brokerFilter={selectedBrokerFilter} />
  {:else if currentTab === "manage"}
    <ImportBatchManager onBatchDeleted={reloadHoldings} />
  {/if}

  <footer>
    <span>DBファイル: {databasePath}</span>
  </footer>
</main>

<style>
  :global(body) {
    margin: 0;
    font-family:
      "Hiragino Kaku Gothic ProN", "Hiragino Sans", "Yu Gothic UI",
      "Meiryo", sans-serif;
    color: #1c2333;
    background: #fbfcfe;
  }
  main {
    padding: 1rem 1.4rem 0.8rem;
    min-height: calc(100vh - 1.8rem);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    border: 3px dashed transparent;
    box-sizing: border-box;
  }
  main.dragging {
    border-color: #3b6fd4;
    background: #eef3fd;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.8rem;
  }
  h1 {
    font-size: 1.25rem;
    margin: 0;
  }
  .import-controls {
    display: flex;
    align-items: center;
    gap: 0.8rem;
  }
  .date-override {
    font-size: 0.85rem;
    color: #445;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  input[type="date"] {
    padding: 0.3rem 0.4rem;
    border: 1px solid #c6ccda;
    border-radius: 6px;
  }
  button {
    padding: 0.5rem 1.1rem;
    border: none;
    border-radius: 8px;
    background: #2455b8;
    color: #fff;
    font-size: 0.92rem;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: #1c479c;
  }
  button:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .tab-bar {
    display: flex;
    gap: 0.2rem;
    border-bottom: 2px solid #d8dce6;
    margin-top: 0.3rem;
  }
  .tab-button {
    padding: 0.5rem 1.3rem;
    border: none;
    border-radius: 8px 8px 0 0;
    background: transparent;
    color: #556;
    font-size: 0.92rem;
    cursor: pointer;
  }
  .tab-button:hover:not(.active) {
    background: #eef1f8;
    color: #223;
  }
  .tab-button.active {
    background: #2455b8;
    color: #fff;
  }
  .broker-filter {
    display: flex;
    gap: 0;
    width: fit-content;
    border: 1px solid #c6ccda;
    border-radius: 8px;
    overflow: hidden;
    margin: 0.4rem 0 0.2rem;
  }
  .filter-button {
    padding: 0.4rem 0.9rem;
    border: none;
    border-radius: 0;
    background: #fff;
    color: #445;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .filter-button + .filter-button {
    border-left: 1px solid #c6ccda;
  }
  .filter-button:hover:not(.active) {
    background: #f2f4f9;
  }
  .filter-button.active {
    background: #2455b8;
    color: #fff;
  }
  .drop-hint {
    margin: 0;
    font-size: 0.82rem;
    color: #778;
  }
  .status-list {
    list-style: none;
    margin: 0.2rem 0;
    padding: 0;
    font-size: 0.85rem;
  }
  .status-list li {
    padding: 0.25rem 0.6rem;
    border-left: 3px solid #2f9e5f;
    background: #f0faf4;
    margin-bottom: 2px;
    border-radius: 0 4px 4px 0;
  }
  .status-list li.error {
    border-left-color: #c0392b;
    background: #fdf1ef;
  }
  footer {
    margin-top: auto;
    padding-top: 0.6rem;
    font-size: 0.75rem;
    color: #99a;
    word-break: break-all;
  }
</style>
