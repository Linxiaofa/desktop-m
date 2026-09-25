import type { FileEntry } from "./api";

/**
 * Every panel in the workspace is a module the user can add or remove, the way
 * DeskBox widgets are created and closed. Two kinds exist:
 *
 * - `workspace` modules carry the daily flow and stay visible.
 * - `settings` modules exist to be configured once. `autoHideWhenReady` lets
 *   them collapse to a one-line summary as soon as they are fully configured,
 *   so a finished setup does not keep occupying the desktop.
 */
export type ModuleId =
  | "files"
  | "plan"
  | "history"
  | "organize"
  | "rules"
  | "providers"
  | "ai";

export type ModuleKind = "workspace" | "settings";

export interface ModuleDef {
  id: ModuleId;
  title: string;
  kicker: string;
  description: string;
  kind: ModuleKind;
  /** Collapses to a summary row once its configuration is complete. */
  autoHideWhenReady: boolean;
  defaultEnabled: boolean;
}

export const MODULES: ModuleDef[] = [
  {
    id: "files",
    title: "桌面文件",
    kicker: "01 / SELECT",
    description: "从 SQLite 索引中选择要移动的文件。",
    kind: "workspace",
    autoHideWhenReady: false,
    defaultEnabled: true,
  },
  {
    id: "plan",
    title: "移动计划",
    kicker: "02 / PLAN & VALIDATE",
    description: "目标目录必须位于当前 Desktop 下。",
    kind: "workspace",
    autoHideWhenReady: false,
    defaultEnabled: true,
  },
  {
    id: "history",
    title: "操作历史",
    kicker: "03 / HISTORY & UNDO",
    description: "每次移动都以事务记录，符合条件时可撤销。",
    kind: "workspace",
    autoHideWhenReady: false,
    defaultEnabled: true,
  },
  {
    id: "organize",
    title: "按规则一键整理",
    kicker: "04 / SMART ORGANIZE",
    description: "用已启用的规则匹配全部桌面文件，先预览分组计划，再决定是否执行。",
    kind: "workspace",
    autoHideWhenReady: false,
    defaultEnabled: true,
  },
  {
    id: "rules",
    title: "规则建议",
    kicker: "05 / RULES",
    description: "按顺序匹配，只建议目标目录，不会自动移动文件。",
    kind: "workspace",
    autoHideWhenReady: false,
    defaultEnabled: true,
  },
  {
    id: "providers",
    title: "AI Provider",
    kicker: "06 / AI PROVIDERS",
    description: "配置模型连接。Provider 不能直接执行文件操作。",
    kind: "settings",
    autoHideWhenReady: true,
    defaultEnabled: true,
  },
  {
    id: "ai",
    title: "AI 整理建议",
    kicker: "07 / AI PLAN",
    description: "先查看发送内容，再由模型生成待验证的移动计划。",
    kind: "workspace",
    autoHideWhenReady: false,
    defaultEnabled: true,
  },
];

export const MODULE_IDS: ModuleId[] = MODULES.map((module) => module.id);

export function moduleDef(id: ModuleId): ModuleDef {
  const found = MODULES.find((module) => module.id === id);
  if (!found) throw new Error(`Unknown module: ${id}`);
  return found;
}

const STORAGE_KEY = "desktop-manager.modules.v1";

/** Reads the saved layout, dropping ids that no longer exist. */
export function loadEnabledModules(): ModuleId[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return MODULES.filter((module) => module.defaultEnabled).map((m) => m.id);
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return MODULES.filter((m) => m.defaultEnabled).map((m) => m.id);
    const known = new Set<string>(MODULE_IDS);
    const restored = parsed.filter(
      (id): id is ModuleId => typeof id === "string" && known.has(id),
    );
    return restored;
  } catch {
    return MODULES.filter((module) => module.defaultEnabled).map((m) => m.id);
  }
}

export function saveEnabledModules(ids: ModuleId[]): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(ids));
  } catch {
    // A full or disabled storage must not break the workspace.
  }
}

/** Per-file icon identity shared with the Rust icon cache. */
const PER_FILE_ICON_EXTENSIONS = new Set([
  "exe",
  "lnk",
  "ico",
  "msi",
  "scr",
  "cpl",
  "url",
]);

export function iconKey(file: FileEntry): string {
  if (file.kind === "directory") return "__dir__";
  const dot = file.name.lastIndexOf(".");
  const extension = dot > 0 ? file.name.slice(dot + 1).toLowerCase() : "";
  // Executables and shortcuts carry their own icon, so they are cached per
  // path; every other type shares one icon per extension.
  return PER_FILE_ICON_EXTENSIONS.has(extension)
    ? `path:${file.path.toLowerCase()}`
    : `ext:${extension}`;
}
