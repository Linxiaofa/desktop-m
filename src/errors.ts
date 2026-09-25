// Core returns precise but English error strings. Map the common ones to
// short Chinese guidance while keeping the original text as a fallback, so a
// new backend message is never hidden.
const HINTS: Array<[RegExp, string]> = [
  [/select 1[–-]100 files/i, "请选择 1–100 个文件。"],
  [/duplicate file selection/i, "存在重复选择的文件，请重新选择。"],
  [/use one ordinary folder name directly under desktop/i, "目标目录必须是 Desktop 下的单层普通文件夹名（不含 \\ / : * ? \" < > |，也不是系统保留名）。"],
  [/outside desktop|path越界/i, "路径超出 Desktop 范围，已拒绝。"],
  [/target exists|target appeared|目标已存在/i, "目标位置已存在同名文件，计划已被阻止。"],
  [/file type, size, or modification time changed|来源已变化/i, "来源文件已变化，请重新扫描并生成计划。"],
  [/plan已执行或失效|already been sent or closed|no longer available/i, "该计划已失效，请重新创建。"],
  [/save a provider api key first/i, "请先为该 Provider 保存 API Key。"],
  [/provider changed after preview/i, "Provider 配置已变更，请重新预览后再发送。"],
  [/ai instruction must not contain a path separator/i, "补充指令中不能包含路径分隔符。"],
  [/ai instruction is too long/i, "补充指令过长（上限 500 字）。"],
  [/ai connection failed|ai response interrupted/i, "AI 连接失败，请检查 Base URL 与网络。"],
  [/ai response is too large/i, "AI 响应过大，已中止请求。"],
  [/ai provider returned http/i, "AI 服务返回错误，请检查模型名称与 Key。"],
  [/ai did not return the required json shape|ai response has no text content/i, "AI 返回内容格式不符合要求，请重试或更换模型。"],
  [/invalid provider base url/i, "Base URL 无效：远程地址必须使用 HTTPS，本机地址需勾选允许 localhost HTTP。"],
  [/api key and clear_api_key cannot both be set/i, "不能同时设置新 Key 与清除 Key。"],
  [/ai provider credentials require windows/i, "当前平台不支持凭据管理器，无法保存 API Key。"],
  [/desktop core lock poisoned/i, "内部状态异常，请重启应用。"],
  [/record not found|provider not found/i, "目标记录不存在，可能已被刷新。"],
];

export function errorMessage(error: unknown): string {
  const raw =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : error && typeof error === "object" && "message" in error
          ? String(error.message)
          : "";
  if (!raw) return "操作失败，请检查 Desktop Core 日志。";
  for (const [pattern, message] of HINTS) {
    if (pattern.test(raw)) return message;
  }
  return raw;
}
