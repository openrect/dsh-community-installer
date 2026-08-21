import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AppState, Locale, LogLine, ServiceState, SetupProgress, UpdateDecision, UpdateState } from "./types";

const mockState: AppState = {
  appVersion: "0.4.6",
  dshVersion: "0.1.0-rc.7",
  nodeVersion: "24.19.0",
  locale: "en-US",
  edition: "online",
  autoCheckDshUpdates: true,
  setupPhase: "not-installed",
  servicePhase: "stopped",
  setupComplete: false,
  progress: 0,
  messageKey: "installCopy",
};

const mockView = new URLSearchParams(window.location.search).get("mock");
if (mockView === "progress") {
  mockState.setupPhase = "dsh";
  mockState.progress = 62;
  mockState.messageKey = "dsh";
} else if (mockView === "failure") {
  mockState.setupPhase = "failed";
  mockState.progress = 78;
  mockState.messageKey = "failed";
} else if (mockView === "startup") {
  mockState.setupComplete = true;
  mockState.setupPhase = "complete";
  mockState.progress = 100;
  mockState.servicePhase = "starting";
} else if (mockView === "ready") {
  mockState.setupComplete = true;
  mockState.setupPhase = "complete";
  mockState.progress = 100;
  mockState.servicePhase = "ready";
} else if (mockView === "update") {
  mockState.setupComplete = true;
  mockState.setupPhase = "complete";
  mockState.progress = 100;
  mockState.servicePhase = "ready";
  mockState.update = {
    target: "dsh",
    phase: "ready",
    version: "0.1.0-rc.8",
    progress: 100,
    messageKey: "dshUpdate",
  };
}

const mockLogs: LogLine[] = [
  { timestamp: "2026-08-19T10:08:21Z", source: "app", line: "Harness service is starting." },
  { timestamp: "2026-08-19T10:08:27Z", source: "stdout", line: "dsh web: http://127.0.0.1:3080" },
];

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export async function command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri()) {
    if (name === "get_app_state") return structuredClone(mockState) as T;
    if (name === "get_recent_logs") return structuredClone(mockLogs) as T;
    return undefined as T;
  }
  return invoke<T>(name, args);
}

export async function subscribe<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<T>(event, ({ payload }) => handler(payload));
}

export const api = {
  state: () => command<AppState>("get_app_state"),
  setup: () => command<void>("begin_setup"),
  openHarness: () => command<void>("open_harness"),
  retry: () => command<void>("retry_service"),
  openLogs: () => command<void>("open_logs"),
  recentLogs: () => command<LogLine[]>("get_recent_logs"),
  checkUpdates: () => command<void>("check_updates", { manual: true }),
  autoCheckDshUpdates: (enabled: boolean) => command<void>("set_auto_check_dsh_updates", { enabled }),
  locale: (locale: Locale) => command<void>("set_locale", { locale }),
  updateDecision: (target: string, version: string, decision: UpdateDecision) =>
    command<void>("respond_to_update", { target, version, decision }),
  dismissUpdate: () => command<void>("dismiss_update_notice"),
  pauseUpdate: () => command<void>("pause_update"),
  showUpdate: () => command<void>("show_update_notice"),
  exit: () => command<void>("exit_harness"),
  requestMainClose: () => command<void>("request_main_close"),
  cancelSetup: () => command<void>("cancel_setup_and_exit"),
  requestExitPrompt: () => command<void>("show_exit_prompt"),
  hideTray: () => command<void>("hide_tray_menu"),
  minimize: async () => {
    if (isTauri()) await getCurrentWindow().minimize();
  },
  closeWindow: async () => {
    if (isTauri()) await getCurrentWindow().hide();
  },
  drag: async () => {
    if (isTauri()) await getCurrentWindow().startDragging();
  },
  onSetup: (handler: (event: SetupProgress) => void) => subscribe("setup://progress", handler),
  onService: (handler: (event: ServiceState) => void) => subscribe("service://state", handler),
  onLog: (handler: (event: LogLine) => void) => subscribe("log://line", handler),
  onUpdate: (handler: (event: UpdateState) => void) => subscribe("update://state", handler),
  onUpdatePrompt: (handler: () => void) => subscribe("ui://update-prompt", handler),
  onExitPrompt: (handler: () => void) => subscribe("ui://exit-prompt", handler),
  onCancelSetup: (handler: () => void) => subscribe("ui://cancel-setup", handler),
  onRefresh: (handler: () => void) => subscribe("ui://refresh", handler),
};
