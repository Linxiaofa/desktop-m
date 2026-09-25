import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  desktopCore,
  type ActionPlan,
  type FileEntry,
  type HistoryEntry,
  type PlanPreview,
  type ScanSummary,
} from "./api";

type BusyAction = "scan" | "refresh" | "preview" | "execute" | `undo:${string}` | null;
type Notice = { kind: "success" | "info"; message: string } | null;

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String(error.message);
  }
  return "操作失败，请检查 Desktop Core 日志。";
}

function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[index]}`;
}

function formatDate(value: string): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function statusLabel(status: string): string {
  const labels: Record<string, string> = {
    committed: "已完成",
    completed: "已完成",
    executed: "已执行",
    undone: "已撤销",
    failed: "失败",
    pending: "待执行",
    partial: "部分完成",
  };
  return labels[status.toLowerCase()] ?? status;
}

function App() {
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(() => new Set());
  const [destination, setDestination] = useState("");
  const [search, setSearch] = useState("");
  const [plan, setPlan] = useState<ActionPlan | null>(null);
  const [preview, setPreview] = useState<PlanPreview | null>(null);
  const [scanSummary, setScanSummary] = useState<ScanSummary | null>(null);
  const [busy, setBusy] = useState<BusyAction>(null);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);

  const visibleFiles = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return files
      .filter((file) =>
        query
          ? file.name.toLocaleLowerCase().includes(query) ||
            file.path.toLocaleLowerCase().includes(query)
          : true,
      )
      .sort((a, b) => a.name.localeCompare(b.name, "zh-CN", { numeric: true }));
  }, [files, search]);

  const selectableFiles = visibleFiles.filter((file) => file.kind === "file");
  const allVisibleSelected =
    selectableFiles.length > 0 && selectableFiles.every((file) => selectedIds.has(file.id));
  const isBusy = busy !== null;
  const canExecute = Boolean(plan && preview?.valid && preview.plan_id === plan.id && !isBusy);

  async function refreshData(): Promise<void> {
    const [nextFiles, nextHistory] = await Promise.all([
      desktopCore.listFiles(),
      desktopCore.listHistory(),
    ]);
    setFiles(nextFiles);
    setHistory(nextHistory);
    setConnected(true);
    const available = new Set(nextFiles.map((file) => file.id));
    setSelectedIds((current) => new Set([...current].filter((id) => available.has(id))));
  }

  async function handleScan(): Promise<void> {
    if (isBusy) return;
    setBusy("scan");
    setError(null);
    setNotice(null);
    try {
      const summary = await desktopCore.scanDesktop();
      setScanSummary(summary);
      await refreshData();
      setNotice({
        kind: "success",
        message: `扫描完成：发现 ${summary.scanned} 项，更新 ${summary.updated} 项，移除 ${summary.removed} 条旧索引。`,
      });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  async function handleRefresh(): Promise<void> {
    if (isBusy) return;
    setBusy("refresh");
    setError(null);
    try {
      await refreshData();
      setNotice({ kind: "info", message: "文件索引和操作历史已刷新。" });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  useEffect(() => {
    void handleScan();
    // Initial scan runs once when the desktop window opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("desktop-index-updated", () => {
      void refreshData().catch((cause) => setError(errorMessage(cause)));
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch((cause) => setError(errorMessage(cause)));
    return () => {
      disposed = true;
      unlisten?.();
    };
    // The event refreshes the authoritative SQLite view after watcher updates.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function invalidatePlan(): void {
    setPlan(null);
    setPreview(null);
    setError(null);
    setNotice(null);
  }

  function toggleFile(id: number): void {
    if (isBusy) return;
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    invalidatePlan();
  }

  function toggleAllVisible(): void {
    if (isBusy || selectableFiles.length === 0) return;
    setSelectedIds((current) => {
      const next = new Set(current);
      if (allVisibleSelected) selectableFiles.forEach((file) => next.delete(file.id));
      else selectableFiles.forEach((file) => next.add(file.id));
      return next;
    });
    invalidatePlan();
  }

  async function handlePreview(): Promise<void> {
    if (isBusy) return;
    if (selectedIds.size === 0) {
      setError("请先选择至少一个桌面文件。");
      return;
    }
    const target = destination.trim();
    if (!target) {
      setError("请填写 Desktop 下的目标目录相对路径。");
      return;
    }

    setBusy("preview");
    setError(null);
    setNotice(null);
    setPlan(null);
    setPreview(null);
    try {
      const created = await desktopCore.createMovePlan([...selectedIds], target);
      setPlan(created);
      const validated = await desktopCore.validatePlan(created.id);
      setPreview(validated);
      if (validated.valid) {
        setNotice({ kind: "success", message: "计划已通过 Desktop Core 策略验证，可以执行。" });
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  async function handleExecute(): Promise<void> {
    if (!canExecute || !plan) return;
    setBusy("execute");
    setError(null);
    setNotice(null);
    try {
      const result = await desktopCore.executePlan(plan.id);
      setPlan(null);
      setPreview(null);
      setSelectedIds(new Set());
      await refreshData();
      setNotice({
        kind: "success",
        message: `事务 ${result.id}：${statusLabel(result.status)}，处理 ${result.executed_count} 项。可在历史中撤销。`,
      });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  async function handleUndo(transactionId: string): Promise<void> {
    if (isBusy) return;
    setBusy(`undo:${transactionId}`);
    setError(null);
    setNotice(null);
    try {
      const result = await desktopCore.undoTransaction(transactionId);
      await refreshData();
      setNotice({
        kind: "success",
        message: `事务 ${result.id}：${statusLabel(result.status)}，恢复 ${result.executed_count} 项。`,
      });
      setPlan(null);
      setPreview(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">DM</span>
          <div>
            <div className="brand-name">Desktop Manager</div>
            <div className="brand-subtitle">安全整理你的 Windows 桌面</div>
          </div>
        </div>
        <div className="header-actions">
          <span className={`connection-pill ${connected ? "connected" : "disconnected"}`}>
            <span className="status-dot" aria-hidden="true" />
            {connected ? "Desktop Core 已连接" : "等待 Desktop Core"}
          </span>
          <button className="button button-quiet" onClick={handleRefresh} disabled={isBusy}>
            {busy === "refresh" ? "刷新中…" : "刷新"}
          </button>
          <button className="button button-primary" onClick={handleScan} disabled={isBusy}>
            {busy === "scan" ? "扫描中…" : "扫描桌面"}
          </button>
        </div>
      </header>

      <main className="app-main">
        <div className="page-intro">
          <div>
            <p className="eyebrow">DESKTOP / WORKSPACE</p>
            <h1>整理桌面，先看清每一步。</h1>
            <p className="intro-copy">选择文件并填写目标目录。移动前会生成计划，经过策略验证后才能执行。</p>
          </div>
          <div className="summary-card" aria-live="polite">
            <span className="summary-label">当前索引</span>
            <strong>{files.length}</strong>
            <span className="summary-foot">项桌面内容</span>
            {scanSummary && <span className="summary-last">最近扫描 {scanSummary.scanned} 项</span>}
          </div>
        </div>

        {error && (
          <div className="message message-error" role="alert">
            <span className="message-icon" aria-hidden="true">!</span>
            <span>{error}</span>
            <button className="message-close" aria-label="关闭错误提示" onClick={() => setError(null)}>×</button>
          </div>
        )}
        {notice && (
          <div className={`message message-${notice.kind}`} role="status">
            <span className="message-icon" aria-hidden="true">✓</span>
            <span>{notice.message}</span>
            <button className="message-close" aria-label="关闭提示" onClick={() => setNotice(null)}>×</button>
          </div>
        )}

        <div className="workspace-grid">
          <section className="panel files-panel" aria-labelledby="files-heading">
            <div className="panel-heading">
              <div>
                <p className="section-kicker">01 / SELECT</p>
                <h2 id="files-heading">桌面文件</h2>
                <p>从 SQLite 索引中选择要移动的文件。</p>
              </div>
              <span className="count-pill">已选 {selectedIds.size}</span>
            </div>
            <div className="file-toolbar">
              <label className="search-field">
                <span aria-hidden="true" className="search-glyph">⌕</span>
                <input
                  type="search"
                  placeholder="按名称或路径搜索"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  aria-label="搜索桌面文件"
                />
              </label>
              <label className="select-all">
                <input
                  type="checkbox"
                  checked={allVisibleSelected}
                  onChange={toggleAllVisible}
                  disabled={isBusy || selectableFiles.length === 0}
                />
                <span>全选当前结果</span>
              </label>
            </div>
            <div className="file-list" aria-busy={busy === "scan" || busy === "refresh"}>
              {visibleFiles.length === 0 ? (
                <div className="empty-state">
                  <div className="empty-symbol" aria-hidden="true">□</div>
                  <strong>{search ? "没有匹配的文件" : "桌面索引为空"}</strong>
                  <span>{search ? "试试其他关键词。" : "点击“扫描桌面”建立索引。"}</span>
                </div>
              ) : (
                visibleFiles.map((file) => (
                  <label key={file.id} className={`file-row ${selectedIds.has(file.id) ? "selected" : ""}`}>
                    <input
                      type="checkbox"
                      checked={selectedIds.has(file.id)}
                      onChange={() => toggleFile(file.id)}
                      disabled={isBusy || file.kind !== "file"}
                      aria-label={`选择 ${file.name}`}
                    />
                    <span className="file-icon" aria-hidden="true">{file.kind === "directory" ? "▣" : "▤"}</span>
                    <span className="file-main">
                      <strong title={file.name}>{file.name}</strong>
                      <small title={file.path}>{file.path}</small>
                    </span>
                    <span className="file-meta">
                      <span>{formatSize(file.size)}</span>
                      <small>{formatDate(file.modified_at)}</small>
                    </span>
                  </label>
                ))
              )}
            </div>
            <div className="panel-footer">文件内容不会发送给 AI；所有移动均由 Desktop Core 执行。</div>
          </section>

          <section className="panel plan-panel" aria-labelledby="plan-heading">
            <div className="panel-heading">
              <div>
                <p className="section-kicker">02 / PLAN & VALIDATE</p>
                <h2 id="plan-heading">移动计划</h2>
                <p>目标目录必须位于当前 Desktop 下。</p>
              </div>
            </div>
            <div className="plan-form">
              <label htmlFor="destination">目标目录（相对 Desktop）</label>
              <div className="destination-field">
                <span className="destination-prefix">Desktop \</span>
                <input
                  id="destination"
                  type="text"
                  value={destination}
                  onChange={(event) => {
                    setDestination(event.target.value);
                    invalidatePlan();
                  }}
                  placeholder="例如：已整理"
                  spellCheck={false}
                  disabled={isBusy}
                />
              </div>
              <p className="field-help">仅填写单层文件夹名称；Desktop Core 会检查路径边界、文件状态及目标冲突。</p>
              <button className="button button-outline plan-submit" onClick={handlePreview} disabled={isBusy || selectedIds.size === 0 || !destination.trim()}>
                {busy === "preview" ? "正在验证…" : "创建计划并预览"}
              </button>
            </div>

            <div className="preview-section">
              <div className="subheading">
                <h3>执行预览</h3>
                {preview && <span className={`validation-badge ${preview.valid ? "valid" : "invalid"}`}>{preview.valid ? "验证通过" : "验证未通过"}</span>}
              </div>
              {!preview ? (
                <div className="preview-empty">计划通过验证后，这里会显示逐项移动路径和策略检查结果。</div>
              ) : (
                <>
                  <div className="preview-list">
                    {preview.items.map((item, index) => (
                      <div className="preview-item" key={`${item.source}-${index}`}>
                        <span className="preview-number">{String(index + 1).padStart(2, "0")}</span>
                        <div>
                          <span title={item.source}>{item.source}</span>
                          <span className="move-arrow">↓</span>
                          <strong title={item.destination}>{item.destination}</strong>
                        </div>
                      </div>
                    ))}
                  </div>
                  {preview.checks.length > 0 && (
                    <div className="validation-details">
                      <h4>策略检查</h4>
                      <ul>{preview.checks.map((check, index) => <li key={`${check}-${index}`}>{check}</li>)}</ul>
                    </div>
                  )}
                  {preview.issues.length > 0 && (
                    <div className="validation-details issue-details">
                      <h4>需要处理</h4>
                      <ul>{preview.issues.map((issue, index) => <li key={`${issue}-${index}`}>{issue}</li>)}</ul>
                    </div>
                  )}
                </>
              )}
            </div>
            <div className="execute-area">
              <button className="button button-primary execute-button" onClick={handleExecute} disabled={!canExecute}>
                {busy === "execute" ? "执行中…" : `执行移动${preview?.valid ? `（${preview.items.length} 项）` : ""}`}
              </button>
              <span>只有通过策略验证的计划才能执行。</span>
            </div>
          </section>
        </div>

        <section className="panel history-panel" aria-labelledby="history-heading">
          <div className="panel-heading history-heading">
            <div>
              <p className="section-kicker">03 / HISTORY & UNDO</p>
              <h2 id="history-heading">操作历史</h2>
              <p>每次移动都以事务记录，符合条件时可撤销。</p>
            </div>
            <span className="count-pill">{history.length} 条记录</span>
          </div>
          {history.length === 0 ? (
            <div className="history-empty">还没有执行过移动事务。</div>
          ) : (
            <div className="history-list">
              {history.map((entry) => (
                <div className="history-row" key={entry.id}>
                  <div className="history-indicator" aria-hidden="true" />
                  <div className="history-info">
                    <strong>{entry.summary}</strong>
                    <span>{formatDate(entry.created_at)} · ID {entry.id}</span>
                  </div>
                  <span className={`history-status status-${entry.status.toLowerCase()}`}>{statusLabel(entry.status)}</span>
                  <button
                    className="button button-small button-outline"
                    onClick={() => void handleUndo(entry.id)}
                    disabled={isBusy || !entry.undo_available}
                    aria-label={`撤销事务 ${entry.id}`}
                  >
                    {busy === `undo:${entry.id}` ? "撤销中…" : "撤销"}
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>
      </main>
      <footer className="app-footer">ActionPlan → Validator → Policy Engine → Transaction Executor → File System</footer>
    </div>
  );
}

export default App;
