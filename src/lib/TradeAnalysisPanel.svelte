<script>
  /**
   * 取引分析: 取引履歴CSV + 譲渡益税明細CSV 由来の確定実現損益を可視化
   * - 実現損益は確定値のみ(e-smart=売買損益列 / SBI=譲渡益税明細)。概算はしない。
   * - 表示期間: 全期間 / 1ヶ月 / 3ヶ月 / 半年 / 1年。start_dateはJS側で暦計算してRustに渡す。
   * props:
   *   brokerFilter: "all" または証券会社識別子('sbi' 等)。変更で自動再取得
   */
  import { invoke } from "@tauri-apps/api/core";
  import * as echarts from "echarts";

  let { brokerFilter = "all" } = $props();

  const periodDefinitions = [
    { key: "all", label: "全期間", months: null },
    { key: "1m", label: "1ヶ月", months: 1 },
    { key: "3m", label: "3ヶ月", months: 3 },
    { key: "6m", label: "半年", months: 6 },
    { key: "1y", label: "1年", months: 12 },
  ];
  let selectedPeriod = $state("all");

  let chartContainer = $state(undefined); // bind:this
  let chartInstance = $state(null);
  let analysis = $state(null);
  let loadErrorMessage = $state("");

  /** 選択期間から開始日('YYYY-MM-DD')を暦計算する。全期間ならnull */
  function computeStartDate(periodKey) {
    const definition = periodDefinitions.find((period) => period.key === periodKey);
    if (!definition || definition.months == null) return null;
    const startDate = new Date();
    startDate.setMonth(startDate.getMonth() - definition.months);
    // ローカルタイムでYYYY-MM-DDを組み立てる(toISOStringはUTCずれの恐れ)
    const year = startDate.getFullYear();
    const month = String(startDate.getMonth() + 1).padStart(2, "0");
    const day = String(startDate.getDate()).padStart(2, "0");
    return `${year}-${month}-${day}`;
  }

  // 証券会社フィルタ or 期間変更のたびに取り直す
  $effect(() => {
    const currentFilter = brokerFilter;
    const currentPeriod = selectedPeriod;
    (async () => {
      try {
        analysis = await invoke("fetch_trade_analysis", {
          brokerFilter: currentFilter === "all" ? null : currentFilter,
          startDate: computeStartDate(currentPeriod),
        });
        loadErrorMessage = "";
      } catch (errorMessage) {
        loadErrorMessage = String(errorMessage);
      }
    })();
  });

  // チャートの初期化と破棄
  $effect(() => {
    if (!chartContainer) return;
    const instance = echarts.init(chartContainer);
    chartInstance = instance;
    const resizeObserver = new ResizeObserver(() => instance.resize());
    resizeObserver.observe(chartContainer);
    return () => {
      resizeObserver.disconnect();
      instance.dispose();
      chartInstance = null;
    };
  });

  function formatYen(value) {
    if (value == null) return "--";
    return `${Math.round(value).toLocaleString("ja-JP")}円`;
  }
  function formatSignedYen(value) {
    if (value == null) return "--";
    return `${value > 0 ? "+" : ""}${formatYen(value)}`;
  }
  function profitLossClass(value) {
    if (value == null) return "";
    if (value > 0) return "profit";
    if (value < 0) return "loss";
    return "";
  }

  const totalTradeCount = $derived(
    analysis ? analysis.buy_trade_count + analysis.sell_trade_count : 0
  );
  const winRateText = $derived.by(() => {
    if (!analysis || analysis.sell_count_with_known_pnl === 0) return "--";
    const rate =
      (analysis.winning_sell_count / analysis.sell_count_with_known_pnl) * 100;
    return `${rate.toFixed(1)}% (${analysis.winning_sell_count}勝/${analysis.sell_count_with_known_pnl}回)`;
  });

  // 月別実現損益(棒) + 累計(折れ線)
  $effect(() => {
    if (!chartInstance || !analysis) return;
    const monthlyPoints = analysis.monthly_points;

    chartInstance.setOption(
      {
        tooltip: { trigger: "axis", valueFormatter: (value) => formatSignedYen(value) },
        legend: { data: ["月別実現損益", "累計"] },
        grid: { left: 80, right: 80, top: 40, bottom: 40 },
        xAxis: { type: "category", data: monthlyPoints.map((point) => point.month) },
        yAxis: [
          {
            type: "value",
            name: "月別",
            axisLabel: { formatter: (value) => value.toLocaleString("ja-JP") },
          },
          {
            type: "value",
            name: "累計",
            axisLabel: { formatter: (value) => value.toLocaleString("ja-JP") },
            splitLine: { show: false },
          },
        ],
        series: [
          {
            name: "月別実現損益",
            type: "bar",
            yAxisIndex: 0,
            data: monthlyPoints.map((point) => ({
              value: point.realized_profit_loss,
              itemStyle: { color: point.realized_profit_loss >= 0 ? "#0a7a3d" : "#c0392b" },
            })),
            barMaxWidth: 46,
          },
          {
            name: "累計",
            type: "line",
            yAxisIndex: 1,
            data: monthlyPoints.map((point) => point.cumulative_total_profit_loss),
            symbolSize: 6,
            color: "#2455b8",
          },
        ],
      },
      { notMerge: true }
    );
  });
</script>

<div class="controls-row">
  <div class="period-switch" role="group" aria-label="表示期間">
    {#each periodDefinitions as period (period.key)}
      <button
        class="period-button"
        class:active={selectedPeriod === period.key}
        onclick={() => (selectedPeriod = period.key)}
      >
        {period.label}
      </button>
    {/each}
  </div>
</div>

{#if loadErrorMessage}
  <p class="error-message">読み込みエラー: {loadErrorMessage}</p>
{/if}

{#if analysis && totalTradeCount === 0}
  <p class="empty-message">
    この期間・条件では取引がありません。取引履歴CSV
    (SBI: SaveFile〜.csv / e-smart: TradeKabu.csv)と、SBIの譲渡益税明細CSV
    (特定口座損益明細)を、保有株CSVと同じようにドラッグ&ドロップまたは
    ファイル選択で取り込めます。
  </p>
{:else if analysis}
  <div class="kpi-row">
    <div class="kpi-card">
      <span class="kpi-label">実現損益(確定)</span>
      <span class="kpi-value {profitLossClass(analysis.total_realized_profit_loss)}">
        {formatSignedYen(analysis.total_realized_profit_loss)}
      </span>
    </div>
    <div class="kpi-card">
      <span class="kpi-label">勝率</span>
      <span class="kpi-value">{winRateText}</span>
      {#if analysis.unknown_pnl_sell_count > 0}
        <span class="kpi-detail">
          損益不明の売り {analysis.unknown_pnl_sell_count}件は除外
          (SBIの譲渡益税明細が未取込)
        </span>
      {/if}
    </div>
    <div class="kpi-card">
      <span class="kpi-label">売買回数</span>
      <span class="kpi-value">
        {totalTradeCount}回
        <span class="kpi-detail">(買 {analysis.buy_trade_count} / 売 {analysis.sell_trade_count})</span>
      </span>
    </div>
    <div class="kpi-card">
      <span class="kpi-label">手数料合計</span>
      <span class="kpi-value">{formatYen(analysis.total_commission)}</span>
    </div>
  </div>

  <div class="chart-container" bind:this={chartContainer}></div>

  <div class="detail-columns">
    <section>
      <h2>銘柄別実現損益</h2>
      {#if analysis.stock_items.length === 0}
        <p class="empty-message">確定した売り取引がまだありません。</p>
      {:else}
        <div class="table-container">
          <table>
            <thead>
              <tr>
                <th>コード</th>
                <th>銘柄名</th>
                <th class="numeric">売り回数</th>
                <th class="numeric">実現損益</th>
              </tr>
            </thead>
            <tbody>
              {#each analysis.stock_items as item (item.stock_code)}
                <tr>
                  <td>{item.stock_code}</td>
                  <td class="stock-name">{item.stock_name}</td>
                  <td class="numeric">{item.sell_trade_count}</td>
                  <td class="numeric {profitLossClass(item.realized_profit_loss)}">
                    {formatSignedYen(item.realized_profit_loss)}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    <section>
      <h2>直近の取引(最大100件)</h2>
      <div class="table-container">
        <table>
          <thead>
            <tr>
              <th>約定日</th>
              <th>銘柄</th>
              <th>売買</th>
              <th class="numeric">数量</th>
              <th class="numeric">単価</th>
              <th class="numeric">実現損益</th>
            </tr>
          </thead>
          <tbody>
            {#each analysis.recent_trades as trade, tradeIndex (tradeIndex)}
              <tr>
                <td class="trade-date">{trade.trade_date}</td>
                <td class="stock-name" title={`${trade.stock_code} ${trade.broker_display_name} ${trade.account_type}`}>
                  {trade.stock_name}
                </td>
                <td>
                  <span class="side-badge {trade.side}">
                    {trade.side === "buy" ? "買" : "売"}
                  </span>
                </td>
                <td class="numeric">{trade.quantity.toLocaleString("ja-JP")}</td>
                <td class="numeric">{trade.price?.toLocaleString("ja-JP") ?? "--"}</td>
                <td class="numeric {profitLossClass(trade.realized_profit_loss)}">
                  {#if trade.realized_profit_loss != null}
                    {formatSignedYen(trade.realized_profit_loss)}
                  {:else if trade.side === "sell"}
                    <span class="unknown-pnl" title="SBIの譲渡益税明細が未取込">不明</span>
                  {:else}
                    --
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  </div>
{/if}

<style>
  .controls-row {
    display: flex;
    align-items: center;
    padding: 0.3rem 0;
  }
  .period-switch {
    display: flex;
    width: fit-content;
    border: 1px solid #c6ccda;
    border-radius: 8px;
    overflow: hidden;
  }
  .period-button {
    padding: 0.4rem 1rem;
    border: none;
    border-radius: 0;
    background: #fff;
    color: #445;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .period-button + .period-button {
    border-left: 1px solid #c6ccda;
  }
  .period-button:hover:not(.active) {
    background: #f2f4f9;
  }
  .period-button.active {
    background: #2455b8;
    color: #fff;
  }
  .kpi-row {
    display: flex;
    gap: 0.8rem;
    flex-wrap: wrap;
    padding: 0.3rem 0;
  }
  .kpi-card {
    flex: 1;
    min-width: 12rem;
    border: 1px solid #d8dce6;
    border-radius: 8px;
    background: #fff;
    padding: 0.6rem 0.9rem;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .kpi-label {
    font-size: 0.75rem;
    color: #778;
  }
  .kpi-value {
    font-size: 1.15rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .kpi-detail {
    font-size: 0.72rem;
    font-weight: 400;
    color: #778;
  }
  .chart-container {
    width: 100%;
    height: 300px;
  }
  .detail-columns {
    display: grid;
    grid-template-columns: 1fr 1.2fr;
    gap: 1rem;
    align-items: start;
  }
  section h2 {
    font-size: 0.95rem;
    margin: 0.4rem 0 0.4rem;
  }
  .table-container {
    overflow: auto;
    max-height: 320px;
    border: 1px solid #d8dce6;
    border-radius: 8px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
    white-space: nowrap;
  }
  th {
    position: sticky;
    top: 0;
    background: #f2f4f9;
    text-align: left;
    padding: 0.5rem 0.7rem;
    border-bottom: 2px solid #d8dce6;
  }
  th.numeric {
    text-align: right;
  }
  td {
    padding: 0.4rem 0.7rem;
    border-bottom: 1px solid #eceef4;
  }
  td.numeric {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  td.stock-name {
    max-width: 11rem;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  td.trade-date {
    color: #556;
  }
  tbody tr:hover {
    background: #f7f9fd;
  }
  .side-badge {
    display: inline-block;
    font-size: 0.75rem;
    border-radius: 4px;
    padding: 0.05rem 0.45rem;
    color: #fff;
  }
  .side-badge.buy {
    background: #2455b8;
  }
  .side-badge.sell {
    background: #e67e22;
  }
  .unknown-pnl {
    color: #99a;
    font-size: 0.8rem;
  }
  .profit {
    color: #0a7a3d;
  }
  .loss {
    color: #c0392b;
  }
  .empty-message {
    color: #667;
    text-align: center;
    padding: 1.5rem 0;
    font-size: 0.9rem;
  }
  .error-message {
    color: #c0392b;
    font-size: 0.88rem;
  }
</style>
