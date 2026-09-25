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
};
