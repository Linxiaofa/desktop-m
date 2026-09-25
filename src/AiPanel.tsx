import { useEffect, useRef, useState } from "react";
import {
  desktopCore,
  type AiPlanResult,
  type AiRequestPreview,
  type ProviderConfig,
} from "./api";
import PanelHeading from "./PanelHeading";
import { moduleDef } from "./modules";

const MODULE = moduleDef("ai");

export default function AiPanel({
  selectedIds,
  onPlanReady,
  onRemove,
}: {
  selectedIds: number[];
  onPlanReady: (result: AiPlanResult) => void;
  onRemove?: () => void;
}) {
  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [providerId, setProviderId] = useState("");
  const [instruction, setInstruction] = useState("");
  const [requestPreview, setRequestPreview] = useState<AiRequestPreview | null>(null);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState<"preview" | "send" | "refresh" | null>(null);
  const [error, setError] = useState("");
  const selectionKey = selectedIds.join(",");
  const selectionKeyRef = useRef(selectionKey);
  selectionKeyRef.current = selectionKey;

  async function reloadProviders() {
    const next = await desktopCore.listProviders();
    setProviders(next);
    setProviderId((current) => current && next.some((item) => item.id === current)
      ? current
      : next[0]?.id ?? "");
  }

  useEffect(() => {
    void reloadProviders().catch((cause) => setError(String(cause)));
  }, []);

  useEffect(() => {
    setRequestPreview(null);
    setReason("");
  }, [selectionKey]);

  async function previewRequest() {
    if (busy || !providerId || selectedIds.length === 0) return;
    setBusy("preview");
    setError("");
    setReason("");
    try {
      setRequestPreview(await desktopCore.previewAiRequest(selectedIds, providerId, instruction));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  }

  async function sendRequest() {
    if (busy || !requestPreview) return;
    const sentSelection = selectionKey;
    setBusy("send");
    setError("");
    try {
      const result = await desktopCore.generateAiPlan(requestPreview.id);
      if (selectionKeyRef.current !== sentSelection) {
        setError("文件选择已变化；AI 计划已丢弃，请重新选择并预览。");
        setRequestPreview(null);
        return;
      }
      setReason(result.reason);
      setRequestPreview(null);
      onPlanReady(result);
    } catch (cause) {
      setError(String(cause));
      setRequestPreview(null);
    } finally {
      setBusy(null);
    }
  }

  async function refreshProviders() {
    if (busy) return;
    setBusy("refresh");
    setError("");
    try {
      await reloadProviders();
      setRequestPreview(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="panel settings-panel" aria-labelledby="ai-heading">
      <PanelHeading
        kicker={MODULE.kicker}
        title={MODULE.title}
        titleId="ai-heading"
        description={MODULE.description}
        badge={<span className="count-pill">已选 {selectedIds.length}</span>}
        onRemove={onRemove}
      />
      <div className="settings-body">
        {error && <div className="inline-error" role="alert">{error}</div>}
        {reason && <div className="inline-notice" role="status">AI 建议：{reason}。计划已在上方等待预览和执行。</div>}
        <div className="ai-form">
          <label>Provider
            <select value={providerId} onChange={(event) => { setProviderId(event.target.value); setRequestPreview(null); }} disabled={busy !== null}>
              {providers.length === 0 && <option value="">请先配置 Provider</option>}
              {providers.map((provider) => <option key={provider.id} value={provider.id}>
                {provider.name} · {provider.model}{provider.hasApiKey ? "" : "（无 Key）"}
              </option>)}
            </select>
          </label>
          <button className="button button-quiet" onClick={() => void refreshProviders()} disabled={busy !== null}>刷新 Provider</button>
          <label className="ai-instruction">补充指令（可选）
            <textarea value={instruction} onChange={(event) => { setInstruction(event.target.value); setRequestPreview(null); }} maxLength={500} placeholder="例如：按项目类型建议一个目录" rows={2} disabled={busy !== null} />
          </label>
          <button className="button button-outline" onClick={() => void previewRequest()} disabled={busy !== null || !providerId || selectedIds.length === 0}>
            {busy === "preview" ? "准备中…" : "预览将发送的信息"}
          </button>
        </div>
        {requestPreview && <div className="ai-request-preview">
          <div className="subheading"><h3>请求摘要</h3><span className="validation-badge valid">等待用户发送</span></div>
          <p>发送给 {requestPreview.providerName} · {requestPreview.model} · {requestPreview.baseUrl}</p>
          <p>仅包含以下选中文件的名称与扩展名，不包含完整路径或文件内容：</p>
          <ul>{requestPreview.files.map((file, index) => <li key={index}>
            {file.name} <span>· {file.extension || "无扩展名"}</span>
          </li>)}</ul>
          <p>补充指令：{requestPreview.instruction || "无"}</p>
          <button className="button button-primary" onClick={() => void sendRequest()} disabled={busy !== null}>
            {busy === "send" ? "正在请求模型…" : "确认发送并生成计划"}
          </button>
        </div>}
      </div>
    </section>
  );
}
