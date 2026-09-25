import { useState } from "react";
import { desktopCore, type OrganizeResult } from "./api";
import { errorMessage } from "./errors";
import PanelHeading from "./PanelHeading";
import { moduleDef } from "./modules";

const MODULE = moduleDef("organize");

type OrganizeBusy = "preview" | "execute" | null;

/**
 * Applies the enabled rules to every indexed Desktop file and turns the result
 * into validated ActionPlans. The panel can execute the runnable groups in
 * sequence, but it never bypasses Core: each group still goes through
 * validate + execute, and every failure stops the batch.
 */
export default function OrganizePanel({
  disabled,
  onBusyChange,
  onExecuted,
  onRemove,
}: {
  disabled: boolean;
  onBusyChange: (busy: boolean) => void;
  onExecuted: (summary: { plans: number; files: number }) => void;
  onRemove?: () => void;
}) {
  const [result, setResult] = useState<OrganizeResult | null>(null);
  const [busy, setBusy] = useState<OrganizeBusy>(null);
  const [error, setError] = useState("");
  const [progress, setProgress] = useState("");

  async function previewOrganize(): Promise<void> {
    if (busy || disabled) return;
    setBusy("preview");
    onBusyChange(true);
    setError("");
    setProgress("");
    try {
      setResult(await desktopCore.organizeByRules());
    } catch (cause) {
      setError(errorMessage(cause));
      setResult(null);
    } finally {
      setBusy(null);
      onBusyChange(false);
    }
  }

  async function executeAll(): Promise<void> {
    if (busy || disabled || !result) return;
    const runnable = result.groups.filter((group) => group.preview.valid);
    if (runnable.length === 0) {
      setError("没有通过验证的分组可以执行。");
      return;
    }
    setBusy("execute");
    onBusyChange(true);
    setError("");
    let plans = 0;
    let files = 0;
    try {
      for (const group of runnable) {
        setProgress(`正在执行 Desktop\\${group.destination}（${group.fileCount} 项）…`);
        const transaction = await desktopCore.executePlan(group.plan.id);
        plans += 1;
        files += transaction.executed_count;
      }
      setResult(null);
      setProgress("");
      onExecuted({ plans, files });
    } catch (cause) {
      setError(errorMessage(cause));
      setProgress("");
      // Report the groups that did succeed before the batch stopped.
      if (plans > 0) onExecuted({ plans, files });
    } finally {
      setBusy(null);
      onBusyChange(false);
    }
  }

  const runnable = result?.groups.filter((group) => group.preview.valid) ?? [];
  const blocked = result?.groups.filter((group) => !group.preview.valid) ?? [];
  const runnableFiles = runnable.reduce((sum, group) => sum + group.fileCount, 0);

  return (
    <section className="panel settings-panel organize-panel" aria-labelledby="organize-heading">
      <PanelHeading
        kicker={MODULE.kicker}
        title={MODULE.title}
        titleId="organize-heading"
        description={MODULE.description}
        badge={<span className="count-pill">{result ? `${result.groups.length} 组` : "未生成"}</span>}
        onRemove={onRemove}
      />
      <div className="settings-body">
        {error && <div className="inline-error" role="alert">{error}</div>}
        {progress && <div className="inline-notice" role="status">{progress}</div>}
        <div className="organize-actions">
          <button
            className="button button-outline"
            onClick={() => void previewOrganize()}
            disabled={disabled || busy !== null}
          >
            {busy === "preview" ? "正在匹配…" : result ? "重新生成整理计划" : "按规则一键整理"}
          </button>
          {result && (
            <button
              className="button button-primary"
              onClick={() => void executeAll()}
              disabled={disabled || busy !== null || runnable.length === 0}
            >
              {busy === "execute"
                ? "正在执行…"
                : `执行全部（${runnable.length} 组 / ${runnableFiles} 项）`}
            </button>
          )}
        </div>

        {!result && (
          <p className="settings-empty">
            整理只会生成计划，不会直接移动文件；执行前仍会逐组验证来源与目标冲突。
          </p>
        )}
        {result && result.groups.length === 0 && (
          <p className="settings-empty">
            没有文件命中已启用的规则。请先在下方「规则建议」中添加规则。
          </p>
        )}

        {result && result.groups.length > 0 && (
          <div className="organize-groups">
            {result.groups.map((group) => (
              <div
                className={`organize-group ${group.preview.valid ? "valid" : "invalid"}`}
                key={group.plan.id}
              >
                <div className="organize-group-head">
                  <strong title={`Desktop\\${group.destination}`}>Desktop\{group.destination}</strong>
                  <span className="organize-count">{group.fileCount} 项</span>
                  <span className={`validation-badge ${group.preview.valid ? "valid" : "invalid"}`}>
                    {group.preview.valid ? "可执行" : "已阻止"}
                  </span>
                </div>
                {group.preview.issues.length > 0 && (
                  <ul className="organize-issues">
                    {group.preview.issues.slice(0, 3).map((issue, index) => (
                      <li key={`${issue}-${index}`}>{issue}</li>
                    ))}
                  </ul>
                )}
              </div>
            ))}
          </div>
        )}

        {result && blocked.length > 0 && (
          <p className="organize-note">
            {blocked.length} 个分组因目标冲突或来源变化被阻止，执行时会自动跳过。
          </p>
        )}
        {result && result.unmatched.length > 0 && (
          <details className="organize-unmatched">
            <summary>{result.unmatched.length} 个文件未命中任何规则（保持原位）</summary>
            <div className="organize-unmatched-list">
              {result.unmatched.slice(0, 100).map((name) => <span key={name}>{name}</span>)}
            </div>
          </details>
        )}
      </div>
    </section>
  );
}
