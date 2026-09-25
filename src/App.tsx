import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  desktopCore,
  type ActionPlan,
  type AiPlanResult,
  type FileEntry,
  type HistoryEntry,
  type PlanPreview,
  type RuleSuggestion,
  type ScanSummary,
} from "./api";
import RulesPanel from "./RulesPanel";
import OrganizePanel from "./OrganizePanel";
import ProviderPanel from "./ProviderPanel";
import AiPanel from "./AiPanel";
import PanelHeading from "./PanelHeading";
import { errorMessage } from "./errors";
import {
  MODULES,
  MODULE_IDS,
  iconKey,
  loadEnabledModules,
  moduleDef,
  saveEnabledModules,
  type ModuleId,
} from "./modules";

type BusyAction = "scan" | "refresh" | "preview" | "suggest" | "execute" | `undo:${string}` | null;
type Notice = { kind: "success" | "info"; message: string } | null;
type SortKey = "name" | "modified" | "size" | "kind";

/** How many icon lookups one IPC round trip may carry. */
const ICON_BATCH = 120;
/** Icons are only fetched for rows the user can actually see. */
const ICON_VISIBLE_LIMIT = 400;

const SORT_LABELS: Record<SortKey, string> = {
  name: "名称",
  modified: "修改时间",
  size: "大小",
  kind: "类型",
};

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
  const [ruleSuggestions, setRuleSuggestions] = useState<RuleSuggestion[]>([]);
  const [busy, setBusy] = useState<BusyAction>(null);
  const [sortKey, setSortKey] = useState<SortKey>("name");
  const [organizeBusy, setOrganizeBusy] = useState(false);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  // Modules the user has created. Nothing is hard-wired into the layout.
  const [enabledModules, setEnabledModules] = useState<ModuleId[]>(loadEnabledModules);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [icons, setIcons] = useState<Record<string, string>>({});
  const requestedIcons = useRef<Set<string>>(new Set());
  const pickerRef = useRef<HTMLDivElement>(null);

  const visibleFiles = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    const filtered = files.filter((file) =>
      query
        ? file.name.toLocaleLowerCase().includes(query) ||
          file.path.toLocaleLowerCase().includes(query)
        : true,
    );
    return filtered.sort((a, b) => {
      switch (sortKey) {
        case "modified":
          return b.modified_at.localeCompare(a.modified_at);
        case "size":
          return b.size - a.size;
        case "kind":
          return (
            a.kind.localeCompare(b.kind) ||
            a.name.localeCompare(b.name, "zh-CN", { numeric: true })
          );
        default:
          return a.name.localeCompare(b.name, "zh-CN", { numeric: true });
      }
    });
  }, [files, search, sortKey]);

  const destinationFolders = useMemo(
    () => files.filter((file) => file.kind === "directory").map((file) => file.name),
    [files],
  );
  const selectableFiles = visibleFiles.filter((file) => file.kind === "file");
  const aiSelectedIds = [...selectedIds].sort((a, b) => a - b);
  const allVisibleSelected =
    selectableFiles.length > 0 && selectableFiles.every((file) => selectedIds.has(file.id));
  const isBusy = busy !== null || organizeBusy;
  const canExecute = Boolean(plan && preview?.valid && preview.plan_id === plan.id && !isBusy);
  const enabled = useMemo(() => new Set(enabledModules), [enabledModules]);
  const availableModules = MODULES.filter((module) => !enabled.has(module.id));

  /** Adds a module if it is missing, keeping registry order for a stable layout. */
  function ensureModule(id: ModuleId): void {
    setEnabledModules((current) =>
      current.includes(id) ? current : MODULE_IDS.filter((each) => each === id || current.includes(each)),
    );
  }

  function addModule(id: ModuleId): void {
    setPickerOpen(false);
    ensureModule(id);
  }

  function removeModule(id: ModuleId): void {
    setEnabledModules((current) => current.filter((each) => each !== id));
  }

  function resetModules(): void {
    setPickerOpen(false);
    setEnabledModules(MODULES.filter((module) => module.defaultEnabled).map((module) => module.id));
  }

  useEffect(() => {
    saveEnabledModules(enabledModules);
  }, [enabledModules]);

  // Real Shell icons are fetched in batches and cached by the backend, so the
  // list only asks once per icon identity.
  useEffect(() => {
    const missing = new Map<string, { key: string; path: string; isDir: boolean }>();
    for (const file of visibleFiles.slice(0, ICON_VISIBLE_LIMIT)) {
      const key = iconKey(file);
      if (icons[key] || requestedIcons.current.has(key)) continue;
      missing.set(key, { key, path: file.path, isDir: file.kind === "directory" });
      if (missing.size >= ICON_BATCH) break;
    }
    if (missing.size === 0) return;
    const batch = [...missing.values()];
    batch.forEach((item) => requestedIcons.current.add(item.key));
    void desktopCore
      .listFileIcons(batch)
      .then((resolved) => setIcons((current) => ({ ...current, ...resolved })))
      .catch(() => {
        // A failed icon lookup must never block the file list.
        batch.forEach((item) => requestedIcons.current.delete(item.key));
      });
  }, [visibleFiles, icons]);

  async function refreshFiles(): Promise<void> {
    const nextFiles = await desktopCore.listFiles();
    setFiles(nextFiles);
    setConnected(true);
    const available = new Set(nextFiles.map((file) => file.id));
    setSelectedIds((current) => new Set([...current].filter((id) => available.has(id))));
  }

  async function refreshHistory(): Promise<void> {
    setHistory(await desktopCore.listHistory());
  }

  async function refreshData(): Promise<void> {
    await Promise.all([refreshFiles(), refreshHistory()]);
  }

  async function handleScan(): Promise<void> {
    if (isBusy) return;
    setBusy("scan");
    setError(null);
    setNotice(null);
    setRuleSuggestions([]);
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
      // A watcher event only changes the file index, not history, so the
      // expensive history/undo checks are skipped here.
      void refreshFiles().catch((cause) => setError(errorMessage(cause)));
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

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 6000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key !== "Escape") return;
      setError(null);
      setNotice(null);
      setPickerOpen(false);
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // The header uses backdrop-filter, which makes it a containing block for
  // fixed children, so the menu closes from a document listener instead of a
  // full-screen overlay.
  useEffect(() => {
    if (!pickerOpen) return;
    function handlePointerDown(event: PointerEvent): void {
      if (!pickerRef.current?.contains(event.target as Node)) setPickerOpen(false);
    }
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [pickerOpen]);

  function invalidatePlan(): void {
    setPlan(null);
    setPreview(null);
    setError(null);
    setNotice(null);
    setRuleSuggestions([]);
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

  async function handleRuleSuggestion(): Promise<void> {
    if (isBusy || selectedIds.size === 0) return;
    setBusy("suggest");
    setError(null);
    try {
      const suggestions = await desktopCore.suggestRules([...selectedIds]);
      setRuleSuggestions(suggestions);
      const first = suggestions[0]?.destination;
      if (first && suggestions.every((item) => item.destination === first)) {
        setDestination(first);
        setPlan(null);
        setPreview(null);
        ensureModule("plan");
        setNotice({ kind: "info", message: "所有选中文件均建议 Desktop\\" + first + "。请创建计划并预览。" });
      } else {
        setNotice({ kind: "info", message: "选中文件没有统一的规则建议，请查看逐项结果或手动填写目录。" });
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  function handleAiPlanReady(result: AiPlanResult): void {
    setDestination(result.destination);
    setPlan(result.plan);
    setPreview(result.preview);
    setRuleSuggestions([]);
    setError(null);
    // A plan the user cannot see would be a dead end, so the module is created
    // on demand before the view scrolls to it.
    ensureModule("plan");
    setNotice({
      kind: "info",
      message: "AI 建议已转为 Desktop Core ActionPlan。请检查上方预览，再决定是否执行。",
    });
    window.requestAnimationFrame(() => {
      document.getElementById("plan-heading")?.scrollIntoView({ behavior: "smooth" });
    });
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

  function handleOrganizeExecuted(summary: { plans: number; files: number }): void {
    setPlan(null);
    setPreview(null);
    setSelectedIds(new Set());
    void refreshData().catch((cause) => setError(errorMessage(cause)));
    setNotice({
      kind: "success",
      message: `已按规则执行 ${summary.plans} 个事务，移动 ${summary.files} 个文件。可在历史中撤销。`,
    });
  }

  async function handleWindowAction(action: "minimize" | "close"): Promise<void> {
    try {
      const window = getCurrentWindow();
      if (action === "minimize") await window.minimize();
      else await window.close();
    } catch (cause) {
      setError(errorMessage(cause));
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
          <div className="module-picker" ref={pickerRef}>
            <button
              className="button button-quiet"
              onClick={() => setPickerOpen((open) => !open)}
              aria-expanded={pickerOpen}
              aria-haspopup="menu"
            >
              ＋ 模块
            </button>
            {pickerOpen && (
              <div className="module-menu" role="menu">
                <p className="module-menu-title">
                  已启用 {enabledModules.length} / {MODULES.length} 个模块
                </p>
                {availableModules.length === 0 ? (
                  <p className="module-menu-empty">全部模块都已显示。</p>
                ) : (
                  availableModules.map((module) => (
                    <button
                      key={module.id}
                      type="button"
                      role="menuitem"
                      className="module-menu-item"
                      onClick={() => addModule(module.id)}
                    >
                      <strong>{module.title}</strong>
                      <small>{module.description}</small>
                      <span className="module-menu-tag">
                        {module.kind === "settings" ? "配置" : "工作"}
                      </span>
                    </button>
                  ))
                )}
                <button type="button" className="module-menu-reset" onClick={resetModules}>
                  恢复默认布局
                </button>
              </div>
            )}
          </div>
          <button className="button button-quiet" onClick={handleRefresh} disabled={isBusy}>
            {busy === "refresh" ? "刷新中…" : "刷新"}
          </button>
          <button className="button button-primary" onClick={handleScan} disabled={isBusy}>
            {busy === "scan" ? "扫描中…" : "扫描桌面"}
          </button>
          <div className="window-actions" aria-label="窗口控制">
            <button className="window-action" type="button" title="最小化" aria-label="最小化 Desktop Manager" onClick={() => void handleWindowAction("minimize")}>−</button>
            <button className="window-action window-action-close" type="button" title="关闭" aria-label="关闭 Desktop Manager" onClick={() => void handleWindowAction("close")}>×</button>
          </div>
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

        {(error || notice) && <div className="message-stack">
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
        </div>}

          {enabled.has("files") && (
          <section className="panel files-panel" aria-labelledby="files-heading">
            <PanelHeading
              kicker={moduleDef("files").kicker}
              title={moduleDef("files").title}
              titleId="files-heading"
              description={moduleDef("files").description}
              badge={<span className="count-pill">已选 {selectedIds.size}</span>}
              onRemove={() => removeModule("files")}
            />
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
              <label className="sort-field">
                <span className="sort-label">排序</span>
                <select
                  value={sortKey}
                  onChange={(event) => setSortKey(event.target.value as SortKey)}
                  aria-label="文件排序方式"
                >
                  {(Object.keys(SORT_LABELS) as SortKey[]).map((key) => (
                    <option key={key} value={key}>{SORT_LABELS[key]}</option>
                  ))}
                </select>
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
                    <span className="file-icon" aria-hidden="true">
                      {icons[iconKey(file)]
                        ? <img src={icons[iconKey(file)]} alt="" width={16} height={16} draggable={false} />
                        : <span className="file-icon-pending" />}
                    </span>
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
          )}

          {enabled.has("plan") && (
          <section className="panel plan-panel" aria-labelledby="plan-heading">
            <PanelHeading
              kicker={moduleDef("plan").kicker}
              title={moduleDef("plan").title}
              titleId="plan-heading"
              description={moduleDef("plan").description}
              onRemove={() => removeModule("plan")}
            />
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
                  list="desktop-folders"
                />
                <datalist id="desktop-folders">
                  {destinationFolders.map((name) => <option key={name} value={name} />)}
                </datalist>
              </div>
              <p className="field-help">仅填写单层文件夹名称；Desktop Core 会检查路径边界、文件状态及目标冲突。输入时会提示 Desktop 下已有的文件夹。</p>
              <button className="button button-quiet rule-suggest-button" onClick={handleRuleSuggestion} disabled={isBusy || selectedIds.size === 0}>
                {busy === "suggest" ? "正在匹配…" : "查看规则建议"}
              </button>
              {ruleSuggestions.length > 0 && <div className="rule-suggestions">
                {ruleSuggestions.map((item) => <div key={item.fileId}>
                  <span>{item.fileName}</span>
                  <strong>{item.destination ? "Desktop\\" + item.destination : "无匹配"}</strong>
                </div>)}
              </div>}
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
          )}

        {enabled.has("history") && (
        <section className="panel history-panel" aria-labelledby="history-heading">
          <PanelHeading
            kicker={moduleDef("history").kicker}
            title={moduleDef("history").title}
            titleId="history-heading"
            description={moduleDef("history").description}
            badge={<span className="count-pill">{history.length} 条记录</span>}
            onRemove={() => removeModule("history")}
          />
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
        )}

        {enabled.has("organize") && (
          <OrganizePanel
            disabled={busy !== null}
            onBusyChange={setOrganizeBusy}
            onExecuted={handleOrganizeExecuted}
            onRemove={() => removeModule("organize")}
          />
        )}
        {enabled.has("rules") && (
          <RulesPanel
            onChanged={() => setRuleSuggestions([])}
            onRemove={() => removeModule("rules")}
          />
        )}
        {enabled.has("providers") && (
          <ProviderPanel onRemove={() => removeModule("providers")} />
        )}
        {enabled.has("ai") && (
          <AiPanel
            selectedIds={aiSelectedIds}
            onPlanReady={handleAiPlanReady}
            onRemove={() => removeModule("ai")}
          />
        )}

        {enabledModules.length === 0 && (
          <div className="workspace-empty">
            <strong>当前没有显示任何模块</strong>
            <span>点击右上角「＋ 模块」创建需要的面板。</span>
            <button className="button button-primary" onClick={resetModules}>恢复默认布局</button>
          </div>
        )}
      </main>
      <footer className="app-footer">ActionPlan → Validator → Policy Engine → Transaction Executor → File System</footer>
    </div>
  );
}

export default App;
