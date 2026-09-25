import { useEffect, useState } from "react";
import {
  desktopCore,
  type ProviderConfig,
  type ProviderInput,
  type ProviderKind,
} from "./api";
import PanelHeading from "./PanelHeading";
import { moduleDef } from "./modules";

const MODULE = moduleDef("providers");

const PRESETS: Record<ProviderKind, { name: string; baseUrl: string; model: string }> = {
  openai: {
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-5.4-mini",
  },
  deepseek: {
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-flash",
  },
  xiaomi_mimo: {
    name: "Xiaomi MiMo",
    baseUrl: "https://api.xiaomimimo.com/v1",
    model: "mimo-v2.6-pro",
  },
  openai_compatible: { name: "OpenAI-compatible", baseUrl: "", model: "" },
  custom: { name: "Custom", baseUrl: "", model: "" },
};

const LABELS: Record<ProviderKind, string> = {
  openai: "OpenAI",
  deepseek: "DeepSeek",
  xiaomi_mimo: "Xiaomi MiMo",
  openai_compatible: "OpenAI-compatible",
  custom: "自定义",
};

type Form = ProviderInput & { apiKeyText: string };

function emptyForm(): Form {
  return {
    kind: "openai",
    ...PRESETS.openai,
    allowLocalHttp: false,
    clearApiKey: false,
    apiKeyText: "",
  };
}

export default function ProviderPanel({ onRemove }: { onRemove?: () => void }) {
  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [form, setForm] = useState<Form>(emptyForm);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  // A settings module that is fully configured collapses to a summary row so a
  // finished setup stops occupying the desktop. "管理" reopens it on demand.
  const [manuallyExpanded, setManuallyExpanded] = useState(false);
  const configured =
    providers.length > 0 && providers.every((provider) => provider.hasApiKey);
  const collapsed = MODULE.autoHideWhenReady && configured && !manuallyExpanded;

  async function reload() {
    setProviders(await desktopCore.listProviders());
  }

  useEffect(() => {
    void reload().catch((cause) => setError(String(cause)));
  }, []);

  function changeKind(kind: ProviderKind) {
    setForm((current) => ({
      ...current,
      kind,
      ...PRESETS[kind],
      allowLocalHttp: false,
    }));
  }

  function edit(provider: ProviderConfig) {
    setForm({
      id: provider.id,
      kind: provider.kind,
      name: provider.name,
      baseUrl: provider.baseUrl,
      model: provider.model,
      allowLocalHttp: provider.allowLocalHttp,
      clearApiKey: false,
      apiKeyText: "",
    });
    setError("");
    setNotice("");
  }

  async function save() {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const input: ProviderInput = {
        id: form.id,
        kind: form.kind,
        name: form.name,
        baseUrl: form.baseUrl,
        model: form.model,
        allowLocalHttp: form.allowLocalHttp,
        clearApiKey: form.clearApiKey,
        apiKey: form.apiKeyText.trim() || undefined,
      };
      await desktopCore.saveProvider(input);
      setForm(emptyForm());
      setManuallyExpanded(false);
      await reload();
      setNotice("Provider 配置已保存。API Key 仅存于 Windows 凭据管理器。");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: string) {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await desktopCore.deleteProvider(id);
      if (form.id === id) setForm(emptyForm());
      setManuallyExpanded(true);
      await reload();
      setNotice("Provider 配置及其凭据已删除。");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel settings-panel" aria-labelledby="providers-heading">
      <PanelHeading
        kicker={MODULE.kicker}
        title={MODULE.title}
        titleId="providers-heading"
        description={MODULE.description}
        badge={<span className="count-pill">{providers.length} 个配置</span>}
        onRemove={onRemove}
      />
      {collapsed ? (
        <div className="settings-collapsed">
          <span className="configured-mark" aria-hidden="true">✓</span>
          <div className="settings-collapsed-text">
            <strong>已完成配置</strong>
            <small>{providers.map((provider) => provider.name).join("、")} · 凭据已保存</small>
          </div>
          <button
            className="button button-small button-outline"
            onClick={() => setManuallyExpanded(true)}
          >
            管理
          </button>
        </div>
      ) : (
      <div className="settings-body">
        {error && <div className="inline-error" role="alert">{error}</div>}
        {notice && <div className="inline-notice" role="status">{notice}</div>}
        <div className="settings-form provider-form">
          <label>类型
            <select value={form.kind} onChange={(event) => changeKind(event.target.value as ProviderKind)}>
              {Object.entries(LABELS).map(([kind, label]) => <option key={kind} value={kind}>{label}</option>)}
            </select>
          </label>
          <label>显示名称
            <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} />
          </label>
          <label className="wide-field">Base URL
            <input type="url" value={form.baseUrl} onChange={(event) => setForm({ ...form, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" spellCheck={false} />
          </label>
          <label>Model
            <input value={form.model} onChange={(event) => setForm({ ...form, model: event.target.value })} spellCheck={false} />
          </label>
          <label>API Key {form.id && <span className="subtle">(留空表示保留现有 Key)</span>}
            <input type="password" value={form.apiKeyText} onChange={(event) => setForm({ ...form, apiKeyText: event.target.value, clearApiKey: false })} placeholder={form.id ? "留空保留现有 Key" : "输入 API Key"} autoComplete="off" />
          </label>
          <label className="check-label wide-field">
            <input type="checkbox" checked={form.allowLocalHttp} onChange={(event) => setForm({ ...form, allowLocalHttp: event.target.checked })} />
            明确允许本机 localhost HTTP（远程地址仍必须使用 HTTPS）
          </label>
          {form.id && <label className="check-label wide-field">
            <input type="checkbox" checked={form.clearApiKey} onChange={(event) => setForm({ ...form, clearApiKey: event.target.checked, apiKeyText: "" })} />
            清除已保存的 API Key
          </label>}
          <div className="form-actions wide-field">
            <button className="button button-primary" onClick={() => void save()} disabled={busy || !form.name.trim() || !form.baseUrl.trim() || !form.model.trim()}>{form.id ? "保存配置" : "添加 Provider"}</button>
            {form.id && <button className="button button-quiet" onClick={() => setForm(emptyForm())} disabled={busy}>取消编辑</button>}
            {configured && <button className="button button-quiet" onClick={() => setManuallyExpanded(false)} disabled={busy}>收起</button>}
          </div>
        </div>
        {providers.length === 0 && <p className="settings-empty">尚未配置 Provider。API Key 不会保存在 SQLite 中。</p>}
        {providers.length > 0 && <div className="settings-list">
          {providers.map((provider) => <div className="settings-row" key={provider.id}>
            <span className="provider-symbol" aria-hidden="true">AI</span>
            <div className="settings-row-main">
              <strong>{provider.name} <span className="subtle">· {LABELS[provider.kind]}</span></strong>
              <small>{provider.model} · {provider.baseUrl} · {provider.hasApiKey ? "凭据已保存" : "未保存凭据"}</small>
            </div>
            <div className="settings-row-actions">
              <button onClick={() => edit(provider)} disabled={busy}>编辑</button>
              <button onClick={() => void remove(provider.id)} disabled={busy}>删除</button>
            </div>
          </div>)}
        </div>}
      </div>
      )}
    </section>
  );
}
