<script>
  /**
   * スナップショット管理画面: 取込履歴の一覧と削除(誤取り込みの取り消し)
   * 削除は取り消し不可なので、実行前に必ず確認ダイアログを挟む。
   * props:
   *   onBatchDeleted: 削除成功時に呼ばれるコールバック(親でholdings等を再取得させる)
   */
  import { invoke } from "@tauri-apps/api/core";

  let { onBatchDeleted = () => {} } = $props();

  let importBatches = $state([]);
  let loadErrorMessage = $state("");
  let pendingDeleteBatchId = $state(null); // 確認待ちの削除対象(nullなら確認モーダル非表示)
  let isDeleting = $state(false);

  export async function reload() {
    try {
      importBatches = await invoke("fetch_import_batches");
      loadErrorMessage = "";
    } catch (errorMessage) {
      loadErrorMessage = String(errorMessage);
    }
  }

  $effect(() => {
    reload();
  });

  function requestDelete(batchId) {
    pendingDeleteBatchId = batchId;
  }

  function cancelDelete() {
    pendingDeleteBatchId = null;
  }

  async function confirmDelete() {
    if (pendingDeleteBatchId == null) return;
    isDeleting = true;
    try {
      await invoke("delete_import_batch", { batchId: pendingDeleteBatchId });
      pendingDeleteBatchId = null;
      await reload();
      onBatchDeleted();
    } catch (errorMessage) {
      loadErrorMessage = String(errorMessage);
    } finally {
      isDeleting = false;
    }
  }

  const targetBatch = $derived(
    importBatches.find((batch) => batch.batch_id === pendingDeleteBatchId) ?? null
  );

  function formatImportedAt(rawDateTimeText) {
    // SQLiteの datetime('now','localtime') 形式 'YYYY-MM-DD HH:MM:SS' をそのまま整形して表示
    return rawDateTimeText.replace("T", " ");
  }
</script>

{#if loadErrorMessage}
  <p class="error-message">読み込みエラー: {loadErrorMessage}</p>
{/if}

{#if importBatches.length === 0 && !loadErrorMessage}
  <p class="empty-message">まだ取込履歴がありません。</p>
{:else}
  <div class="table-container">
    <table>
      <thead>
        <tr>
          <th>取込日(スナップショット日)</th>
          <th>証券会社</th>
          <th class="numeric">銘柄数</th>
          <th>元ファイル</th>
          <th>取込日時</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each importBatches as batch (batch.batch_id)}
          <tr>
            <td>{batch.snapshot_date}</td>
            <td>{batch.broker_display_name}</td>
            <td class="numeric">{batch.holding_count}</td>
            <td class="file-name" title={batch.source_file_name ?? ""}>
              {batch.source_file_name ?? "--"}
            </td>
            <td class="imported-at">{formatImportedAt(batch.imported_at)}</td>
            <td>
              <button class="delete-button" onclick={() => requestDelete(batch.batch_id)}>
                削除
              </button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

{#if targetBatch}
  <div class="modal-overlay" role="dialog" aria-modal="true">
    <div class="modal-box">
      <h2>取込履歴を削除しますか?</h2>
      <p>
        <strong>{targetBatch.broker_display_name} / {targetBatch.snapshot_date}</strong>
        ({targetBatch.holding_count}銘柄分)を削除します。
      </p>
      <p class="warning-text">この操作は取り消せません。</p>
      <div class="modal-actions">
        <button class="cancel-button" onclick={cancelDelete} disabled={isDeleting}>
          キャンセル
        </button>
        <button class="confirm-delete-button" onclick={confirmDelete} disabled={isDeleting}>
          {isDeleting ? "削除中..." : "削除する"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .empty-message {
    color: #667;
    text-align: center;
    padding: 3rem 0;
  }
  .error-message {
    color: #c0392b;
    font-size: 0.88rem;
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
  }
  th {
    position: sticky;
    top: 0;
    background: #f2f4f9;
    text-align: left;
    padding: 0.55rem 0.7rem;
    border-bottom: 2px solid #d8dce6;
  }
  th.numeric {
    text-align: right;
  }
  td {
    padding: 0.45rem 0.7rem;
    border-bottom: 1px solid #eceef4;
  }
  td.numeric {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  td.file-name {
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #667;
    font-size: 0.82rem;
  }
  td.imported-at {
    color: #778;
    font-size: 0.8rem;
    white-space: nowrap;
  }
  tbody tr:hover {
    background: #f7f9fd;
  }
  .delete-button {
    padding: 0.3rem 0.8rem;
    border: 1px solid #d99;
    border-radius: 6px;
    background: #fff;
    color: #c0392b;
    font-size: 0.8rem;
    cursor: pointer;
  }
  .delete-button:hover {
    background: #fdf1ef;
  }

  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(20, 25, 40, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
  }
  .modal-box {
    background: #fff;
    border-radius: 10px;
    padding: 1.4rem 1.6rem;
    max-width: 26rem;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.2);
  }
  .modal-box h2 {
    margin: 0 0 0.6rem;
    font-size: 1.05rem;
  }
  .modal-box p {
    margin: 0.3rem 0;
    font-size: 0.9rem;
  }
  .warning-text {
    color: #c0392b;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    margin-top: 1rem;
  }
  .cancel-button {
    padding: 0.5rem 1.1rem;
    border: 1px solid #c6ccda;
    border-radius: 8px;
    background: #fff;
    color: #445;
    font-size: 0.88rem;
    cursor: pointer;
  }
  .cancel-button:hover:not(:disabled) {
    background: #f2f4f9;
  }
  .confirm-delete-button {
    padding: 0.5rem 1.1rem;
    border: none;
    border-radius: 8px;
    background: #c0392b;
    color: #fff;
    font-size: 0.88rem;
    cursor: pointer;
  }
  .confirm-delete-button:hover:not(:disabled) {
    background: #a12e21;
  }
  button:disabled {
    opacity: 0.6;
    cursor: wait;
  }
</style>
