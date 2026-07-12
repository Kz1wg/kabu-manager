<script>
  /**
   * 資産推移グラフ(ECharts)
   * - 左軸: 評価額合計(面付き折れ線)
   * - 右軸: 評価損益合計(折れ線)
   * props:
   *   brokerFilter: "all" または証券会社識別子('sbi' 等)。変更で自動再取得
   */
  import { invoke } from "@tauri-apps/api/core";
  import * as echarts from "echarts";

  let { brokerFilter = "all" } = $props();

  let chartContainer = $state(undefined); // bind:this(条件付き描画でも安全なよう$stateにする)
  let chartInstance = $state(null);
  let historyPoints = $state([]);
  let loadErrorMessage = $state("");

  // フィルタ変更のたびにRust側から集計済みデータを取り直す
  $effect(() => {
    const currentFilter = brokerFilter;
    (async () => {
      try {
        historyPoints = await invoke("fetch_asset_history", {
          brokerFilter: currentFilter === "all" ? null : currentFilter,
        });
        loadErrorMessage = "";
      } catch (errorMessage) {
        loadErrorMessage = String(errorMessage);
      }
    })();
  });

  // チャートの初期化と破棄(コンテナサイズ変化にも追従)
  $effect(() => {
    // コンテナ要素がまだ描画されていない(条件ブロック外)間は何もしない。
    // chartContainerは$stateなので、要素がバインドされた時点でこのeffectが再実行される
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
    return `${Math.round(value).toLocaleString("ja-JP")}円`;
  }

  // データが変わったら描画を更新
  $effect(() => {
    if (!chartInstance) return;
    const snapshotDates = historyPoints.map((point) => point.snapshot_date);
    const marketValues = historyPoints.map((point) => point.total_market_value);
    const profitLossValues = historyPoints.map((point) => point.total_profit_loss);

    chartInstance.setOption(
      {
        tooltip: {
          trigger: "axis",
          valueFormatter: formatYen,
        },
        legend: { data: ["評価額", "評価損益"] },
        grid: { left: 90, right: 80, top: 40, bottom: 60 },
        xAxis: {
          type: "category",
          data: snapshotDates,
          boundaryGap: false,
        },
        yAxis: [
          {
            type: "value",
            name: "評価額",
            scale: true, // 0起点にせず変動を見やすく
            axisLabel: { formatter: (value) => value.toLocaleString("ja-JP") },
          },
          {
            type: "value",
            name: "評価損益",
            axisLabel: { formatter: (value) => value.toLocaleString("ja-JP") },
            splitLine: { show: false },
          },
        ],
        dataZoom: [{ type: "inside" }, { type: "slider", height: 18 }],
        series: [
          {
            name: "評価額",
            type: "line",
            yAxisIndex: 0,
            data: marketValues,
            symbolSize: 6,
            lineStyle: { width: 2.5 },
            areaStyle: { opacity: 0.08 },
            color: "#2455b8",
          },
          {
            name: "評価損益",
            type: "line",
            yAxisIndex: 1,
            data: profitLossValues,
            symbolSize: 6,
            color: "#e67e22",
          },
        ],
      },
      { notMerge: true }
    );
  });
</script>

{#if loadErrorMessage}
  <p class="error-message">読み込みエラー: {loadErrorMessage}</p>
{/if}

{#if historyPoints.length === 0 && !loadErrorMessage}
  <p class="empty-message">
    まだ推移データがありません。日々のCSVを取り込むと折れ線が伸びていきます。
  </p>
{/if}

<!-- チャート本体(データ0件でも領域は確保しておく) -->
<div class="chart-container" bind:this={chartContainer}></div>

{#if historyPoints.length === 1}
  <p class="hint-message">
    データが1日分のみのため点のみ表示されています。翌営業日以降のCSVを取り込むと線になります。
  </p>
{/if}

<style>
  .chart-container {
    width: 100%;
    height: 460px;
  }
  .empty-message,
  .hint-message {
    color: #667;
    text-align: center;
    margin: 0.6rem 0;
    font-size: 0.88rem;
  }
  .error-message {
    color: #c0392b;
    font-size: 0.88rem;
  }
</style>
