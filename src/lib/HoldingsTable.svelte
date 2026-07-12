<script>
  /**
   * 保有一覧テーブル(列ヘッダークリックでソート可能)
   * props:
   *   holdings: Rust側 fetch_latest_holdings の戻り値の配列
   */
  let { holdings = [] } = $props();

  let sortColumnKey = $state("stock_code");
  let sortAscending = $state(true);

  const columnDefinitions = [
    { key: "stock_code", label: "コード", numeric: false },
    { key: "stock_name", label: "銘柄名", numeric: false },
    { key: "broker_display_name", label: "証券会社", numeric: false },
    { key: "account_type", label: "口座区分", numeric: false },
    { key: "sector_33", label: "業種", numeric: false },
    { key: "quantity", label: "数量", numeric: true },
    { key: "average_acquisition_price", label: "取得単価", numeric: true },
    { key: "current_price", label: "現在値", numeric: true },
    { key: "market_value", label: "評価額", numeric: true },
    { key: "profit_loss", label: "評価損益", numeric: true },
    { key: "profit_loss_rate", label: "損益率(%)", numeric: true },
  ];

  function toggleSort(columnKey) {
    if (sortColumnKey === columnKey) {
      sortAscending = !sortAscending;
    } else {
      sortColumnKey = columnKey;
      sortAscending = true;
    }
  }

  const sortedHoldings = $derived(
    [...holdings].sort((leftRecord, rightRecord) => {
      const leftValue = leftRecord[sortColumnKey];
      const rightValue = rightRecord[sortColumnKey];
      // null/undefined は常に末尾へ
      if (leftValue == null && rightValue == null) return 0;
      if (leftValue == null) return 1;
      if (rightValue == null) return -1;
      const comparison =
        typeof leftValue === "number"
          ? leftValue - rightValue
          : String(leftValue).localeCompare(String(rightValue), "ja");
      return sortAscending ? comparison : -comparison;
    })
  );

  const totalMarketValue = $derived(
    holdings.reduce((sum, record) => sum + (record.market_value ?? 0), 0)
  );
  const totalProfitLoss = $derived(
    holdings.reduce((sum, record) => sum + (record.profit_loss ?? 0), 0)
  );

  function formatNumber(value, fractionDigits = 0) {
    if (value == null) return "--";
    return value.toLocaleString("ja-JP", {
      minimumFractionDigits: 0,
      maximumFractionDigits: fractionDigits,
    });
  }

  function profitLossClass(value) {
    if (value == null) return "";
    if (value > 0) return "profit";
    if (value < 0) return "loss";
    return "";
  }
</script>

{#if holdings.length === 0}
  <p class="empty-message">
    まだデータがありません。CSVファイルを取り込んでください。
  </p>
{:else}
  <div class="summary-bar">
    <span>評価額合計: <strong>{formatNumber(totalMarketValue)} 円</strong></span>
    <span class={profitLossClass(totalProfitLoss)}>
      評価損益合計: <strong>{totalProfitLoss > 0 ? "+" : ""}{formatNumber(totalProfitLoss)} 円</strong>
    </span>
    <span class="row-count">{holdings.length} 件</span>
  </div>

  <div class="table-container">
    <table>
      <thead>
        <tr>
          {#each columnDefinitions as column}
            <th
              class:numeric={column.numeric}
              onclick={() => toggleSort(column.key)}
            >
              {column.label}
              {#if sortColumnKey === column.key}
                <span class="sort-indicator">{sortAscending ? "▲" : "▼"}</span>
              {/if}
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each sortedHoldings as record (record.broker_display_name + record.account_type + record.stock_code)}
          <tr>
            <td>{record.stock_code}</td>
            <td class="stock-name">{record.stock_name}</td>
            <td>{record.broker_display_name}</td>
            <td>{record.account_type}</td>
            <td>{record.sector_33 ?? "--"}</td>
            <td class="numeric">{formatNumber(record.quantity)}</td>
            <td class="numeric">{formatNumber(record.average_acquisition_price, 1)}</td>
            <td class="numeric">{formatNumber(record.current_price, 1)}</td>
            <td class="numeric">{formatNumber(record.market_value)}</td>
            <td class="numeric {profitLossClass(record.profit_loss)}">
              {record.profit_loss > 0 ? "+" : ""}{formatNumber(record.profit_loss)}
            </td>
            <td class="numeric {profitLossClass(record.profit_loss_rate)}">
              {record.profit_loss_rate > 0 ? "+" : ""}{formatNumber(record.profit_loss_rate, 2)}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .empty-message {
    color: #667;
    text-align: center;
    padding: 3rem 0;
  }
  .summary-bar {
    display: flex;
    gap: 2rem;
    align-items: baseline;
    padding: 0.6rem 0.2rem;
    font-size: 0.95rem;
  }
  .summary-bar .row-count {
    margin-left: auto;
    color: #778;
    font-size: 0.85rem;
  }
  .table-container {
    overflow: auto;
    border: 1px solid #d8dce6;
    border-radius: 8px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.88rem;
    white-space: nowrap;
  }
  th {
    position: sticky;
    top: 0;
    background: #f2f4f9;
    text-align: left;
    padding: 0.55rem 0.7rem;
    cursor: pointer;
    user-select: none;
    border-bottom: 2px solid #d8dce6;
  }
  th:hover {
    background: #e6eaf4;
  }
  th.numeric {
    text-align: right;
  }
  .sort-indicator {
    font-size: 0.7rem;
    color: #556;
  }
  td {
    padding: 0.45rem 0.7rem;
    border-bottom: 1px solid #eceef4;
  }
  td.numeric {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  td.stock-name {
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  tbody tr:hover {
    background: #f7f9fd;
  }
  .profit {
    color: #0a7a3d;
  }
  .loss {
    color: #c0392b;
  }
</style>
