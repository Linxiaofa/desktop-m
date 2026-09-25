import { useEffect, useState } from "react";
import { desktopCore, type MatchType, type Rule, type RuleInput } from "./api";
import PanelHeading from "./PanelHeading";
import { moduleDef } from "./modules";

const EMPTY_FORM: RuleInput = {
  matchType: "extension",
  pattern: "",
  destination: "",
  enabled: true,
};

const MODULE = moduleDef("rules");

export default function RulesPanel({
  onChanged,
  onRemove,
}: {
  onChanged: () => void;
  onRemove?: () => void;
}) {
  const [rules, setRules] = useState<Rule[]>([]);
  const [form, setForm] = useState<RuleInput>(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [loaded, setLoaded] = useState(false);

  async function reload() {
    const items = await desktopCore.listRules();
    setRules(items);
    setLoaded(true);
  }

  useEffect(() => {
    void reload().catch((cause) => setError(String(cause)));
  }, []);

  async function save() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await desktopCore.saveRule(form);
      await reload();
      setForm(EMPTY_FORM);
      onChanged();
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
    try {
      await desktopCore.deleteRule(id);
      await reload();
      if (form.id === id) setForm(EMPTY_FORM);
      onChanged();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function move(index: number, offset: number) {
    const next = [...rules];
    const other = index + offset;
    if (busy || other < 0 || other >= next.length) return;
    [next[index], next[other]] = [next[other], next[index]];
    setBusy(true);
    setError("");
    try {
      setRules(await desktopCore.reorderRules(next.map((rule) => rule.id)));
      onChanged();
    } catch (cause) {
      setError(String(cause));
      await reload();
    } finally {
      setBusy(false);
    }
  }

  function edit(rule: Rule) {
    setForm({
      id: rule.id,
      matchType: rule.matchType,
      pattern: rule.pattern,
      destination: rule.destination,
      enabled: rule.enabled,
    });
  }

  return (
    <section className="panel settings-panel" aria-labelledby="rules-heading">
      <PanelHeading
        kicker={MODULE.kicker}
        title={MODULE.title}
        titleId="rules-heading"
        description={MODULE.description}
        badge={<span className="count-pill">{rules.length} 条规则</span>}
        onRemove={onRemove}
      />
      <div className="settings-body">
        {error && <div className="inline-error" role="alert">{error}</div>}
        <div className="settings-form rule-form">
          <label>匹配方式
            <select value={form.matchType} onChange={(event) => setForm({ ...form, matchType: event.target.value as MatchType })}>
              <option value="extension">扩展名</option>
              <option value="name">文件名（* 和 ?）</option>
            </select>
          </label>
          <label>匹配内容
            <input value={form.pattern} onChange={(event) => setForm({ ...form, pattern: event.target.value })} placeholder={form.matchType === "extension" ? "例如 pdf" : "例如 报告*.pdf"} />
          </label>
          <label>建议目录
            <input value={form.destination} onChange={(event) => setForm({ ...form, destination: event.target.value })} placeholder="Desktop 下单层目录" />
          </label>
          <label className="check-label">
            <input type="checkbox" checked={form.enabled} onChange={(event) => setForm({ ...form, enabled: event.target.checked })} />
            启用
          </label>
          <div className="form-actions">
            <button className="button button-primary" onClick={() => void save()} disabled={busy || !form.pattern.trim() || !form.destination.trim()}>{form.id ? "保存规则" : "添加规则"}</button>
            {form.id && <button className="button button-quiet" onClick={() => setForm(EMPTY_FORM)} disabled={busy}>取消编辑</button>}
          </div>
        </div>
        {loaded && rules.length === 0 && <p className="settings-empty">暂无规则。添加后可为选中的文件查看建议。</p>}
        {rules.length > 0 && <div className="settings-list">
          {rules.map((rule, index) => (
            <div className="settings-row" key={rule.id}>
              <span className="rule-order">{index + 1}</span>
              <div className="settings-row-main">
                <strong>{rule.matchType === "extension" ? "扩展名" : "文件名"} · {rule.pattern}</strong>
                <small>→ Desktop\{rule.destination} · {rule.enabled ? "启用" : "停用"}</small>
              </div>
              <div className="settings-row-actions">
                <button title="上移" onClick={() => void move(index, -1)} disabled={busy || index === 0}>↑</button>
                <button title="下移" onClick={() => void move(index, 1)} disabled={busy || index === rules.length - 1}>↓</button>
                <button onClick={() => edit(rule)} disabled={busy}>编辑</button>
                <button onClick={() => void remove(rule.id)} disabled={busy}>删除</button>
              </div>
            </div>
          ))}
        </div>}
      </div>
    </section>
  );
}
