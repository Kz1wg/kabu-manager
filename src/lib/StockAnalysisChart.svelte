<script>
  /**
   * 銘柄別分析: 銘柄を選択して評価額・評価損益の推移を表示
   * - 銘柄リストは全履歴由来(売却済み銘柄も「売却済み」グループで選択可)
   * - 複数の証券会社で同一銘柄を保有している場合は合算(brokerFilterで絞り込み可)
   * - ツールチップに数量・現在値も表示
   * props:
   *   brokerFilter: "all" または証券会社識別子('sbi' 等)。変更で自動再取得
   */
  import { invoke } from "@tauri-apps/api/core";
  import * as echarts from "echarts";

  let { brokerFilter = "all" } = $props();

  let chartContainer = $state(undefined); // bind:this(条件付き描画でも安全なよう$stateにする)
  let chartInstance = $state(null);
  let stockList = $state([]);
  let selectedStockCode = $state("");
  let historyPoints = $state([]);
  let loadErrorMessage = $state("");

  const currentlyHeldStocks = $derived(
    stockList.filter((stock) => stock.is_currently_held)
  );
  const soldOutStocks = $derived(
    stockList.filter((stock) => !stock.is_currently_held)
  );
  const selectedStock = $derived(
    stockList.find((stock) => stock.stock_code === selectedStockCode) ?? null
  );

  // 証券会社フィルタに連動して銘柄リストを取り直す。
  // 選択中の銘柄が新しいリストに無ければ(その証券会社で保有履歴が無い)、
  // 先頭の保有中銘柄に自動で選び直す。
  $effect(() => {
    const currentFilter = brokerFilter;
    (async () => {
      try {
        const fetchedStockList = await invoke("fetch_stock_list", {
          brokerFilter: currentFilter === "all" ? null : currentFilter,
        });
        stockList = fetchedStockList;

        const selectionStillAvailable = fetchedStockList.some(
          (stock) => stock.stock_code === selectedStockCode
        );
        if (!selectionStillAvailable) {
          const firstHeldStock = fetchedStockList.find(
            (stock) => stock.is_currently_held
          );
          selectedStockCode =
            (firstHeldStock ?? fetchedStockList[0])?.stock_code ?? "";
        }
      } catch (errorMessage) {
        loadErrorMessage = String(errorMessage);
      }
    })();
  });

  // 銘柄またはフィルタが変わるたびに推移を取り直す
  $effect(() => {
    const currentStockCode = selectedStockCode;
    const currentFilter = brokerFilter;
    if (!currentStockCode) return;
    (async () => {
      try {
        historyPoints = await invoke("fetch_stock_history", {
          stockCode: currentStockCode,
          brokerFilter: currentFilter === "all" ? null : currentFilter,
        });
        loadErrorMessage = "";
      } catch (errorMessage) {
        loadErrorMessage = String(errorMessage);
      }
    })();
  });

  // チャートの初期化と破棄
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
    const points = historyPoints;

    chartInstance.setOption(
      {
        tooltip: {
          trigger: "axis",
          // 評価額・損益に加えて数量・現在値も出す
          formatter: (seriesParams) => {
            const point = points[seriesParams[0].dataIndex];
            if (!point) return "";
            const lines = [
              `<strong>${point.snapshot_date}</strong>`,
              `数量: ${point.total_quantity.toLocaleString("ja-JP")}株`,
              point.current_price != null
                ? `現在値: ${point.current_price.toLocaleString("ja-JP")}円`
                : null,
              `評価額: ${formatYen(point.total_market_value)}`,
              `評価損益: ${point.total_profit_loss > 0 ? "+" : ""}${formatYen(point.total_profit_loss)}`,
            ];
            return lines.filter(Boolean).join("<br/>");
          },
        },
        legend: { data: ["評価額", "評価損益"] },
        grid: { left: 90, right: 80, top: 40, bottom: 60 },
        xAxis: {
          type: "category",
          data: points.map((point) => point.snapshot_date),
          boundaryGap: false,
        },
        yAxis: [
          {
            type: "value",
            name: "評価額",
            scale: true,
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
            data: points.map((point) => point.total_market_value),
            symbolSize: 6,
            lineStyle: { width: 2.5 },
            areaStyle: { opacity: 0.08 },
            color: "#2455b8",
          },
          {
            name: "評価損益",
            type: "line",
            yAxisIndex: 1,
            data: points.map((point) => point.total_profit_loss),
            symbolSize: 6,
            color: "#e67e22",
          },
        ],
      },
      { notMerge: true }
    );
  });
</script>

<div class="stock-selector">
  <label>
    銘柄:
    <select bind:value={selectedStockCode}>
      {#if currentlyHeldStocks.length > 0}
        <optgroup label="保有中">
          {#each currentlyHeldStocks as stock (stock.stock_code)}
            <option value={stock.stock_code}>
              {stock.stock_code} {stock.stock_name}
            </option>
          {/each}
        </optgroup>
      {/if}
      {#if soldOutStocks.length > 0}
        <optgroup label="売却済み">
          {#each soldOutStocks as stock (stock.stock_code)}
            <option value={stock.stock_code}>
              {stock.stock_code} {stock.stock_name}
            </option>
          {/each}
        </optgroup>
      {/if}
    </select>
  </label>
  {#if selectedStock?.sector_33}
    <span class="sector-badge">{selectedStock.sector_33}</span>
  {/if}
</div>

{#if loadErrorMessage}
  <p class="error-message">読み込みエラー: {loadErrorMessage}</p>
{/if}

{#if selectedStockCode && historyPoints.length === 0 && !loadErrorMessage}
  <p class="empty-message">
    この条件(証券会社フィルタ)ではデータがありません。
  </p>
{/if}

<div class="chart-container" bind:this={chartContainer}></div>

{#if historyPoints.length === 1}
  <p class="hint-message">
    データが1日分のみのため点のみ表示されています。日々のCSVを取り込むと線になります。
  </p>
{/if}

<style>
  .stock-selector {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    padding: 0.3rem 0;
    font-size: 0.9rem;
  }
  select {
    padding: 0.35rem 0.5rem;
    border: 1px solid #c6ccda;
    border-radius: 6px;
    font-size: 0.9rem;
    min-width: 16rem;
    background: #fff;
  }
  .sector-badge {
    font-size: 0.78rem;
    color: #2455b8;
    background: #e8eefb;
    border-radius: 999px;
    padding: 0.15rem 0.7rem;
  }
  .chart-container {
    width: 100%;
    height: 440px;
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
