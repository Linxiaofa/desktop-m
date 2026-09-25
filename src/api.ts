import { invoke } from "@tauri-apps/api/core";

export interface ScanSummary {
  scanned: number;
  updated: number;
  removed: number;
}

export interface FileEntry {
  id: number;
  path: string;
  name: string;
  kind: string;
  size: number;
  modified_at: string;
}

export interface PlanItem {
  file_id: number;
  source: string;
  destination: string;
}

export interface ActionPlan {
  id: string;
  items: PlanItem[];
  status: string;
}

export interface PlanPreview {
  plan_id: string;
  valid: boolean;
  checks: string[];
  issues: string[];
  items: Array<{ source: string; destination: string }>;
}

export interface TransactionResult {
  id: string;
  status: string;
  executed_count: number;
}

export interface HistoryEntry {
  id: string;
  created_at: string;
  status: string;
  summary: string;
  undo_available: boolean;
}

export type MatchType = "extension" | "name";
export interface Rule {
  id: string;
  matchType: MatchType;
  pattern: string;
  destination: string;
  enabled: boolean;
  priority: number;
}
export interface RuleInput {
  id?: string;
  matchType: MatchType;
  pattern: string;
  destination: string;
  enabled: boolean;
}
export interface RuleSuggestion {
  fileId: number;
  fileName: string;
  destination: string | null;
  ruleId: string | null;
}

export type ProviderKind =
  | "openai"
  | "deepseek"
  | "xiaomi_mimo"
  | "openai_compatible"
  | "custom";
export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  name: string;
  baseUrl: string;
  model: string;
  allowLocalHttp: boolean;
  hasApiKey: boolean;
  createdAt: string;
  updatedAt: string;
}
export interface ProviderInput {
  id?: string;
  kind: ProviderKind;
  name: string;
  baseUrl: string;
  model: string;
  allowLocalHttp: boolean;
  apiKey?: string;
  clearApiKey: boolean;
}

export interface AiFile {
  name: string;
  extension: string;
}
export interface AiRequestPreview {
  id: string;
  providerId: string;
  providerName: string;
  model: string;
  baseUrl: string;
  files: AiFile[];
  instruction: string;
}
export interface AiPlanResult {
  plan: ActionPlan;
  preview: PlanPreview;
  destination: string;
  reason: string;
}

export interface IconRequest {
  key: string;
  path: string;
  isDir: boolean;
}

export interface OrganizeGroup {
  destination: string;
  fileCount: number;
  plan: ActionPlan;
  preview: PlanPreview;
}

export interface OrganizeResult {
  groups: OrganizeGroup[];
  unmatched: string[];
  totalFiles: number;
  matchedFiles: number;
}

export const desktopCore = {
  scanDesktop: () => invoke<ScanSummary>("scan_desktop"),
  listFiles: () => invoke<FileEntry[]>("list_files"),
  createMovePlan: (fileIds: number[], destination: string) =>
    invoke<ActionPlan>("create_move_plan", { fileIds, destination }),
  validatePlan: (planId: string) =>
    invoke<PlanPreview>("validate_plan", { planId }),
  executePlan: (planId: string) =>
    invoke<TransactionResult>("execute_plan", { planId }),
  listHistory: () => invoke<HistoryEntry[]>("list_history"),
  undoTransaction: (transactionId: string) =>
    invoke<TransactionResult>("undo_transaction", { transactionId }),
  listRules: () => invoke<Rule[]>("list_rules"),
  saveRule: (input: RuleInput) => invoke<Rule>("save_rule", { input }),
  deleteRule: (id: string) => invoke<void>("delete_rule", { id }),
  reorderRules: (ids: string[]) => invoke<Rule[]>("reorder_rules", { ids }),
  suggestRules: (fileIds: number[]) =>
    invoke<RuleSuggestion[]>("suggest_rules", { fileIds }),
  organizeByRules: () => invoke<OrganizeResult>("organize_by_rules"),
  listFileIcons: (requests: IconRequest[]) =>
    invoke<Record<string, string>>("list_file_icons", { requests }),
  listProviders: () => invoke<ProviderConfig[]>("list_providers"),
  saveProvider: (input: ProviderInput) =>
    invoke<ProviderConfig>("save_provider", { input }),
  deleteProvider: (id: string) => invoke<void>("delete_provider", { id }),
  previewAiRequest: (fileIds: number[], providerId: string, instruction: string) =>
    invoke<AiRequestPreview>("preview_ai_request", { fileIds, providerId, instruction }),
  generateAiPlan: (requestId: string) =>
    invoke<AiPlanResult>("generate_ai_plan", { requestId }),
};
