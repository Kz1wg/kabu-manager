<script>
  /**
   * 構成比: 最新スナップショットの評価額構成を可視化
   * - ツリーマップ: セクター → 銘柄 の階層(クリックでズームイン)
   * - 円グラフ: セクター別 / 銘柄別
   * セクターは stock_master.sector_33(SBIのCSV取り込みで自動更新)。
   * 未登録の銘柄は「未分類」に入る。
   * props:
   *   brokerFilter: "all" または証券会社識別子('sbi' 等)。変更で自動再取得
   */
  import { invoke } from "@tauri-apps/api/core";
  import * as echarts from "echarts";

  let { brokerFilter = "all" } = $props();

  const viewDefinitions = [
    { key: "treemap", label: "ツリーマップ" },
    { key: "pie_sector", label: "円グラフ(セクター別)" },
    { key: "pie_stock", label: "円グラフ(銘柄別)" },
  ];
  let currentView = $state("treemap");

  let chartContainer = $state(undefined); // bind:this(条件付き描画でも安全なよう$stateにする)
  let chartInstance = $state(null);
  let compositionItems = $state([]);
  let loadErrorMessage = $state("");

  const UNCLASSIFIED_SECTOR_LABEL = "未分類";

  // フィルタ変更のたびに構成データを取り直す
  $effect(() => {
    const currentFilter = brokerFilter;
    (async () => {
      try {
        compositionItems = await invoke("fetch_composition", {
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

  const totalMarketValue = $derived(
    compositionItems.reduce((sum, item) => sum + item.total_market_value, 0)
  );

  function formatYen(value) {
    return `${Math.round(value).toLocaleString("ja-JP")}円`;
  }

  function formatPercent(value) {
    if (totalMarketValue === 0) return "0%";
    return `${((value / totalMarketValue) * 100).toFixed(1)}%`;
  }

  /** セクター単位に畳み込む: [{ name, value, children: [...] }] */
  function buildSectorTree(items) {
    const stocksBySector = new Map();
    for (const item of items) {
      const sectorName = item.sector_33 ?? UNCLASSIFIED_SECTOR_LABEL;
      if (!stocksBySector.has(sectorName)) {
        stocksBySector.set(sectorName, []);
      }
      stocksBySector.get(sectorName).push({
        name: item.stock_name,
        value: item.total_market_value,
      });
    }
    return [...stocksBySector.entries()]
      .map(([sectorName, stockNodes]) => ({
        name: sectorName,
        value: stockNodes.reduce((sum, node) => sum + node.value, 0),
        children: stockNodes,
      }))
      .sort((left, right) => right.value - left.value);
  }

  function buildTreemapOption(items) {
    return {
      tooltip: {
        formatter: (params) =>
          `${params.name}<br/>${formatYen(params.value)} (${formatPercent(params.value)})`,
      },
      series: [
        {
          type: "treemap",
          data: buildSectorTree(items),
          leafDepth: 2, // セクター内の銘柄まで一度に表示。セクタークリックでズームイン
          roam: false,
          label: {
            formatter: (params) => `${params.name}\n${formatPercent(params.value)}`,
            fontSize: 12,
          },
          upperLabel: {
            show: true,
            height: 24,
            formatter: (params) => `${params.name} ${formatPercent(params.value)}`,
          },
          breadcrumb: { show: true, bottom: 4 },
          levels: [
            { itemStyle: { borderColor: "#fbfcfe", borderWidth: 3, gapWidth: 3 } },
            {
              colorSaturation: [0.3, 0.55],
              itemStyle: { borderColorSaturation: 0.6, gapWidth: 1 },
            },
          ],
        },
      ],
    };
  }

  function buildPieOption(items, groupBySector) {
    const pieData = groupBySector
      ? buildSectorTree(items).map((sectorNode) => ({
          name: sectorNode.name,
          value: sectorNode.value,
        }))
      : items
          .map((item) => ({ name: item.stock_name, value: item.total_market_value }))
          .sort((left, right) => right.value - left.value);

    return {
      tooltip: {
        formatter: (params) =>
          `${params.name}<br/>${formatYen(params.value)} (${formatPercent(params.value)})`,
      },
      legend: { type: "scroll", orient: "vertical", right: 8, top: 20, bottom: 20 },
      series: [
        {
          type: "pie",
          radius: ["35%", "68%"], // ドーナツ型
          center: ["42%", "50%"],
          data: pieData,
          label: {
            formatter: (params) => `${params.name}\n${formatPercent(params.value)}`,
            fontSize: 11,
          },
          emphasis: {
            itemStyle: { shadowBlur: 8, shadowColor: "rgba(0,0,0,0.25)" },
          },
        },
      ],
    };
  }

  // データまたは表示切り替えのたびに描画を更新
  $effect(() => {
    if (!chartInstance) return;
    const option =
      currentView === "treemap"
        ? buildTreemapOption(compositionItems)
        : buildPieOption(compositionItems, currentView === "pie_sector");
    chartInstance.setOption(option, { notMerge: true });
  });
</script>

<div class="view-controls">
  <div class="view-switch" role="group" aria-label="表示形式">
    {#each viewDefinitions as view (view.key)}
      <button
        class="view-button"
        class:active={currentView === view.key}
        onclick={() => (currentView = view.key)}
      >
        {view.label}
      </button>
    {/each}
  </div>
  {#if compositionItems.length > 0}
    <span class="total-label">評価額合計: {formatYen(totalMarketValue)}</span>
  {/if}
</div>

{#if loadErrorMessage}
  <p class="error-message">読み込みエラー: {loadErrorMessage}</p>
{/if}

{#if compositionItems.length === 0 && !loadErrorMessage}
  <p class="empty-message">
    この条件(証券会社フィルタ)ではデータがありません。
  </p>
{/if}

<div class="chart-container" bind:this={chartContainer}></div>

<style>
  .view-controls {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.3rem 0;
  }
  .view-switch {
    display: flex;
    width: fit-content;
    border: 1px solid #c6ccda;
    border-radius: 8px;
    overflow: hidden;
  }
  .view-button {
    padding: 0.4rem 0.9rem;
    border: none;
    border-radius: 0;
    background: #fff;
    color: #445;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .view-button + .view-button {
    border-left: 1px solid #c6ccda;
  }
  .view-button:hover:not(.active) {
    background: #f2f4f9;
  }
  .view-button.active {
    background: #2455b8;
    color: #fff;
  }
  .total-label {
    font-size: 0.88rem;
    color: #445;
  }
  .chart-container {
    width: 100%;
    height: 470px;
  }
  .empty-message {
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
