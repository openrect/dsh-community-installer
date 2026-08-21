export type Locale = "en-US" | "zh-CN";
export type Edition = "online" | "offline";
export type SetupPhase = "not-installed" | "preparing" | "node" | "dsh" | "validating" | "complete" | "failed";
export type ServicePhase = "stopped" | "starting" | "stopping" | "ready" | "failed";
export type UpdateTarget = "controller" | "dsh";
export type UpdateDecision = "install" | "later" | "skip";
export type UpdatePhase = "checking" | "available" | "ready" | "current" | "failed" | "installing";

export interface AppState {
  appVersion: string;
  dshVersion: string;
  nodeVersion: string;
  locale: Locale;
  edition: Edition;
  autoCheckDshUpdates: boolean;
  setupPhase: SetupPhase;
  servicePhase: ServicePhase;
  setupComplete: boolean;
  progress: number;
  messageKey: string;
  update?: UpdateState;
}

export interface SetupProgress {
  phase: SetupPhase;
  percent: number;
  messageKey: string;
  detail?: string;
  resolvedItems?: number;
  reusedItems?: number;
  downloadedItems?: number;
  addedItems?: number;
  totalItems?: number;
  elapsedSeconds?: number;
}

export interface ServiceState {
  phase: ServicePhase;
  messageKey: string;
}

export interface LogLine {
  timestamp: string;
  source: "app" | "stdout" | "stderr";
  line: string;
}

export interface UpdateState {
  target: UpdateTarget;
  phase: UpdatePhase;
  version?: string;
  progress?: number;
  resolvedItems?: number;
  reusedItems?: number;
  downloadedItems?: number;
  addedItems?: number;
  totalItems?: number;
  elapsedSeconds?: number;
  messageKey: string;
}
