import "./styles.css";
import { startAmbientCanvas } from "./ambient";
import { api } from "./bridge";
import { translate, type MessageKey } from "./i18n";
import type { AppState, Locale, LogLine, ServiceState, SetupProgress, UpdateState } from "./types";

const appElement = document.querySelector<HTMLDivElement>("#app");
const canvasElement = document.querySelector<HTMLCanvasElement>("#ambient");
if (!appElement || !canvasElement) throw new Error("Application root is missing.");
const app = appElement;
const canvas = canvasElement;

const logoUrl = new URL("../assets/dsh-modern-mark.svg", import.meta.url).href;
const query = new URLSearchParams(window.location.search);
const view = query.get("view") ?? "main";
let state: AppState;
try {
  state = await api.state();
} catch (error) {
  const chinese = navigator.language.toLowerCase().startsWith("zh");
  const title = chinese ? "界面未能加载" : "The interface could not load";
  const copy = chinese ? "请重新打开 DSH Community Installer。" : "Please reopen DSH Community Installer.";
  if (view !== "tray") startAmbientCanvas(canvas);
  else canvas.hidden = true;
  app.innerHTML = view === "tray"
    ? `<div class="window-shell compact"><div class="tray"><div class="tray-version">${title}</div></div></div>`
    : `<div class="ambient-veil"></div><div class="window-shell"><main class="main-stage"><section class="surface"><h1>${title}</h1><p class="copy">${copy}</p></section></main></div>`;
  console.error("Failed to load application state.", error);
  throw error;
}
const requestedPrompt = query.get("prompt");
let promptKind: "update" | "update-result" | "exit" | "cancel-setup" = requestedPrompt === "cancel-setup"
  ? "cancel-setup"
  : requestedPrompt === "exit"
    ? "exit"
  : requestedPrompt === "update" || state.update?.phase === "ready" || state.update?.phase === "available" ? "update" : "exit";

class SubscriptionScope {
  private readonly unsubscribers: Array<() => void> = [];

  get active(): boolean {
    return this.unsubscribers.length > 0;
  }

  add(unsubscribe: () => void): void {
    this.unsubscribers.push(unsubscribe);
  }

  dispose(): void {
    this.unsubscribers.splice(0).forEach((unsubscribe) => unsubscribe());
  }
}

const logSubscriptions = new SubscriptionScope();

if (view !== "tray") startAmbientCanvas(canvas);
else canvas.hidden = true;

function text(key: MessageKey, values: Record<string, string> = {}): string {
  return translate(state.locale, key, values);
}

function escape(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", "\"": "&quot;",
  })[character] ?? character);
}

const minimizeIcon = `<svg viewBox="0 0 14 14" aria-hidden="true"><path d="M3 7.5h8"/></svg>`;
const closeIcon = `<svg viewBox="0 0 14 14" aria-hidden="true"><path d="m4 4 6 6m0-6-6 6"/></svg>`;

function languagePicker(): string {
  return `<div class="language-picker" aria-label="Language">
    <button type="button" data-locale="zh-CN" class="${state.locale === "zh-CN" ? "active" : ""}">中文</button>
    <button type="button" data-locale="en-US" class="${state.locale === "en-US" ? "active" : ""}">EN</button>
  </div>`;
}

function titlebar(options: { language?: boolean; title?: string; minimize?: boolean } = {}): string {
  return `<header class="titlebar" data-drag-region>
    <div class="drag-region" data-drag-region>
      ${options.title ? `<h2>${escape(options.title)}</h2>` : `<img class="logo" src="${logoUrl}" alt="DSH" />`}
    </div>
    ${options.language ? languagePicker() : ""}
    <div class="window-actions">
      ${options.minimize === false ? "" : `<button class="icon-button" id="minimize" aria-label="${text("minimize")}">${minimizeIcon}</button>`}
      <button class="icon-button" id="close-window" aria-label="${text("close")}">${closeIcon}</button>
    </div>
  </header>`;
}

function shell(content: string, titleOptions: Parameters<typeof titlebar>[0] = { language: true }): string {
  return `<div class="ambient-veil"></div><div class="window-shell">${titlebar(titleOptions)}<main class="main-stage">${content}</main></div>`;
}

function isIndeterminateSetup(messageKey: string): boolean {
  return messageKey !== "node";
}

function progressSurface(title: string, copy: string, percent: number, actions = "", indeterminate = false): string {
  const safePercent = Math.max(0, Math.min(100, percent));
  return `<section class="surface" data-setup-progress>
    <h1 id="setup-progress-title">${escape(title)}</h1>
    <p class="copy" id="setup-progress-copy">${escape(copy)}</p>
    <div class="progress-track${indeterminate ? " indeterminate" : ""}" id="setup-progress-track"><div class="progress-bar" id="setup-progress-bar" style="width:${safePercent}%"></div></div>
    <div class="progress-value" id="setup-progress-value">${indeterminate ? text("working") : `${Math.round(safePercent)}%`}</div>
    ${actions}
  </section>`;
}

function setupCopy(phase: SetupProgress["phase"]): string {
  return phase === "preparing" ? text("installCopy") : text("setupWorkingCopy");
}

function patchSetupProgress(event: SetupProgress): boolean {
  const surface = document.querySelector<HTMLElement>("[data-setup-progress]");
  const title = document.querySelector<HTMLElement>("#setup-progress-title");
  const copy = document.querySelector<HTMLElement>("#setup-progress-copy");
  const track = document.querySelector<HTMLElement>("#setup-progress-track");
  const bar = document.querySelector<HTMLElement>("#setup-progress-bar");
  const value = document.querySelector<HTMLElement>("#setup-progress-value");
  if (!surface || !title || !copy || !track || !bar || !value) return false;
  const indeterminate = isIndeterminateSetup(event.messageKey);
  title.textContent = text(event.messageKey as MessageKey);
  copy.textContent = setupCopy(event.phase);
  track.classList.toggle("indeterminate", indeterminate);
  bar.style.width = `${Math.max(0, Math.min(100, event.percent))}%`;
  value.textContent = indeterminate ? text("working") : `${Math.round(event.percent)}%`;
  return true;
}

function renderMain(): void {
  let content: string;
  if (!state.setupComplete && state.setupPhase === "not-installed") {
    content = `<section class="surface">
      <h1>${text("installTitle")}</h1>
      <p class="copy">${text("installCopy")}</p>
      <button class="primary wide" id="begin-setup">${text("install")}</button>
    </section>`;
  } else if (!state.setupComplete && state.setupPhase === "failed") {
    const failure = failureCopy();
    content = progressSurface(failure.title, failure.copy, state.progress,
      `<div class="center-actions"><button class="secondary" id="open-logs">${text("openLogs")}</button><button class="primary" id="retry-setup">${text("retry")}</button></div>`);
  } else if (!state.setupComplete) {
    const key = (state.messageKey || state.setupPhase) as MessageKey;
    content = progressSurface(text(key), setupCopy(state.setupPhase), state.progress, "", isIndeterminateSetup(state.messageKey));
  } else if (state.servicePhase === "failed") {
    const failure = failureCopy();
    content = progressSurface(failure.title, failure.copy, 100,
      `<div class="center-actions"><button class="secondary" id="open-logs">${text("openLogs")}</button><button class="primary" id="retry-service">${text("retry")}</button></div>`);
  } else if (state.servicePhase === "ready") {
    content = `<section class="surface">
      <div class="success-mark">✓</div>
      <h1>${text("complete")}</h1>
      <p class="copy">${text("completeCopy")}</p>
      <button class="primary wide" id="open-harness">${text("openHarness")}</button>
    </section>`;
  } else {
    content = progressSurface(text("startupTitle"), text("startupCopy"), 74);
  }
  app.innerHTML = shell(content, { language: true });
  bindWindowControls();
  bindLanguageControls();
  bind("begin-setup", () => api.setup());
  bind("retry-setup", () => api.setup());
  bind("retry-service", () => api.retry());
  bind("open-logs", () => api.openLogs());
  bind("open-harness", () => api.openHarness());
}

function failureCopy(): { title: string; copy: string } {
  if (state.messageKey === "portInUse") {
    return { title: text("portInUse"), copy: text("portInUseCopy") };
  }
  if (state.messageKey === "startupTimedOut") {
    return { title: text("startupTimedOut"), copy: text("startupTimedOutCopy") };
  }
  return { title: text("failed"), copy: text("failedCopy") };
}

function renderTray(): void {
  const serviceClass = state.servicePhase === "ready" ? "ready" : state.servicePhase === "starting" || state.servicePhase === "stopping" ? "starting" : state.servicePhase === "failed" ? "failed" : "";
  const updateLabel = (state.update?.phase === "ready" || state.update?.phase === "available") && state.update.version
    ? text("installUpdate", { version: state.update.version })
    : state.update?.phase === "checking" ? text("checkingUpdates") : text("checkUpdates");
  app.innerHTML = `<div class="window-shell compact">
    <div class="tray">
      <div class="tray-version"><span class="status-dot ${serviceClass}"></span> Harness ${escape(state.dshVersion)}</div>
      <div class="tray-divider"></div>
      <button class="tray-item" id="tray-open">${text("openHarness")}</button>
      <button class="tray-item" id="tray-logs">${text("openLogs")}</button>
      <div class="tray-divider"></div>
      <button class="tray-item" id="tray-updates">${updateLabel}</button>
      <button class="tray-item tray-toggle" id="tray-auto"><span>${text("autoUpdates")}</span><span class="check">${state.autoDownload ? "✓" : ""}</span></button>
      <div class="locale-toggle" aria-label="${text("language")}">
        <button data-locale="zh-CN" class="${state.locale === "zh-CN" ? "active" : ""}">中文</button>
        <button data-locale="en-US" class="${state.locale === "en-US" ? "active" : ""}">EN</button>
      </div>
      <div class="tray-divider"></div>
      <button class="tray-item" id="tray-exit">${text("exitHarness")}</button>
    </div>
  </div>`;
  bind("tray-open", async () => { await api.openHarness(); await api.hideTray(); });
  bind("tray-logs", async () => { await api.openLogs(); await api.hideTray(); });
  bind("tray-updates", async () => {
    await api.hideTray();
    if ((state.update?.phase === "ready" || state.update?.phase === "available") && state.update.version) {
      await api.updateDecision(state.update.target, state.update.version, "install");
    } else {
      await api.checkUpdates();
    }
  });
  bind("tray-auto", async () => { await api.autoDownload(!state.autoDownload); state = await api.state(); renderTray(); });
  bind("tray-exit", async () => { await api.requestExitPrompt(); await api.hideTray(); });
  bindLanguageControls(renderTray);
}

async function renderLogs(): Promise<void> {
  const initial = await api.recentLogs();
  app.innerHTML = `<div class="ambient-veil"></div><div class="window-shell">
    ${titlebar({ title: text("logTitle"), language: false })}
    <main class="logs"><pre class="log-output" id="log-output"></pre></main>
  </div>`;
  bindWindowControls();
  const output = document.querySelector<HTMLElement>("#log-output");
  if (!output) return;
  const append = (entry: LogLine) => {
    const line = `[${entry.timestamp}] [${entry.source}] ${entry.line}\n`;
    output.append(document.createTextNode(line));
    output.scrollTop = output.scrollHeight;
  };
  if (initial.length === 0) output.textContent = text("noLogs");
  else initial.forEach(append);
  if (!logSubscriptions.active) {
    logSubscriptions.add(await api.onLog((entry) => {
      const currentOutput = document.querySelector<HTMLElement>("#log-output");
      if (!currentOutput) return;
      if (currentOutput.textContent === text("noLogs")) currentOutput.textContent = "";
      const line = `[${entry.timestamp}] [${entry.source}] ${entry.line}\n`;
      currentOutput.append(document.createTextNode(line));
      currentOutput.scrollTop = currentOutput.scrollHeight;
    }));
  }
}

function renderPrompt(): void {
  let content: string;
  if (promptKind === "cancel-setup") {
    content = `<section class="surface">
      <h1>${text("cancelSetupTitle")}</h1>
      <p class="copy">${text("cancelSetupCopy")}</p>
      <div class="button-row">
        <button class="secondary" id="continue-setup">${text("continueSetup")}</button>
        <button class="danger-neutral" id="cancel-setup">${text("cancelSetup")}</button>
      </div>
    </section>`;
  } else if (promptKind === "update" && (state.update?.phase === "ready" || state.update?.phase === "available") && state.update.version) {
    const update = state.update;
    const version = update.version!;
    const copy = update.target === "controller" ? text("controllerUpdate") : text("dshUpdate");
    content = `<section class="surface">
      <p class="eyebrow">${update.target === "controller" ? "DSH Community Installer" : "DSH"}</p>
      <h1>${text(update.phase === "available" ? "updateAvailable" : "updateReady", { version: escape(version) })}</h1>
      <p class="copy">${copy}</p>
      <div class="button-row equal">
        <button class="primary" data-update="install">${text("installNow")}</button>
        <button class="secondary" data-update="later">${text("later")}</button>
        <button class="secondary" data-update="skip">${text("skip")}</button>
      </div>
    </section>`;
  } else if (promptKind === "update-result" && state.update?.phase === "current") {
    content = `<section class="surface">
      <h1>${text("upToDate")}</h1>
      <p class="copy">${text("upToDateCopy", { appVersion: escape(state.appVersion), dshVersion: escape(state.dshVersion) })}</p>
      <div class="button-row"><button class="primary" id="dismiss-update">${text("close")}</button></div>
    </section>`;
  } else if (promptKind === "update-result" && state.update?.phase === "failed") {
    const policyBlocked = state.update.messageKey === "scriptPolicyBlocked";
    const nodeVersionBlocked = state.update.messageKey === "nodeVersionBlocked";
    const installFailure = state.update.messageKey === "updateInstallFailed" || policyBlocked || nodeVersionBlocked;
    content = `<section class="surface">
      <h1>${text(policyBlocked ? "scriptPolicyBlocked" : nodeVersionBlocked ? "nodeVersionBlocked" : installFailure ? "updateInstallFailed" : "updateCheckFailed")}</h1>
      <p class="copy">${text(policyBlocked ? "scriptPolicyBlockedCopy" : nodeVersionBlocked ? "nodeVersionBlockedCopy" : installFailure ? "updateInstallFailedCopy" : "updateCheckFailedCopy")}</p>
      <div class="button-row">
        <button class="secondary" id="dismiss-update">${text("close")}</button>
        ${installFailure ? `<button class="primary" id="open-update-logs">${text("openLogs")}</button>` : `<button class="primary" id="retry-update">${text("retry")}</button>`}
      </div>
    </section>`;
  } else if (promptKind === "update-result" && state.update?.phase === "installing") {
    content = progressSurface(text("installingUpdate"), text("setupWorkingCopy"), 50, "", true);
  } else {
    content = `<section class="surface">
      <h1>${text("exitTitle")}</h1>
      <p class="copy">${text("exitCopy")}</p>
      <div class="button-row">
        <button class="secondary" id="cancel-exit">${text("cancel")}</button>
        <button class="danger-neutral" id="confirm-exit">${text("exit")}</button>
      </div>
    </section>`;
  }
  app.innerHTML = shell(content, { language: false, minimize: false });
  bindWindowControls();
  bind("continue-setup", () => api.closeWindow());
  bind("cancel-setup", async () => {
    await api.closeWindow();
    await api.cancelSetup();
  });
  bind("cancel-exit", () => api.closeWindow());
  bind("confirm-exit", () => api.exit());
  bind("dismiss-update", () => api.dismissUpdate());
  bind("retry-update", async () => {
    await api.dismissUpdate();
    await api.checkUpdates();
  });
  bind("open-update-logs", async () => {
    await api.dismissUpdate();
    await api.openLogs();
  });
  document.querySelectorAll<HTMLButtonElement>("[data-update]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!state.update?.version) return;
      await runAction(button, () => api.updateDecision(state.update!.target, state.update!.version!, button.dataset.update as "install" | "later" | "skip"));
    });
  });
}

function bind(id: string, action: () => void | Promise<void>): void {
  const button = document.querySelector<HTMLButtonElement>(`#${id}`);
  button?.addEventListener("click", () => void runAction(button, action));
}

async function runAction(button: HTMLButtonElement, action: () => void | Promise<void>): Promise<void> {
  if (button.disabled) return;
  button.disabled = true;
  try {
    await action();
  } catch (error) {
    console.error("Action failed.", error);
    state = await api.state().catch(() => state);
    renderCurrent();
  } finally {
    if (button.isConnected) button.disabled = false;
  }
}

function bindWindowControls(): void {
  bind("minimize", () => api.minimize());
  bind("close-window", () => view === "main"
    ? api.requestMainClose()
    : view === "prompt" && promptKind === "update-result"
      ? api.dismissUpdate()
      : api.closeWindow());
  document.querySelectorAll<HTMLElement>("[data-drag-region]").forEach((element) => {
    element.addEventListener("mousedown", (event) => {
      if (event.button === 0 && event.target === element) void api.drag();
    });
  });
}

function bindLanguageControls(after = renderCurrent): void {
  document.querySelectorAll<HTMLButtonElement>("[data-locale]").forEach((button) => {
    button.addEventListener("click", async () => {
      const locale = button.dataset.locale as Locale;
      await api.locale(locale);
      state.locale = locale;
      after();
    });
  });
}

function renderCurrent(): void {
  if (view === "tray") renderTray();
  else if (view === "prompt") renderPrompt();
  else if (view === "logs") void renderLogs();
  else renderMain();
}

await api.onSetup((event: SetupProgress) => {
  state.setupPhase = event.phase;
  state.progress = event.percent;
  state.messageKey = event.messageKey;
  state.setupComplete = event.phase === "complete";
  if (view !== "main") return;
  if (event.phase === "failed" || event.phase === "complete" || !patchSetupProgress(event)) {
    renderMain();
  }
});
await api.onService((event: ServiceState) => {
  state.servicePhase = event.phase;
  state.messageKey = event.messageKey;
  if (view === "main") renderMain();
  else if (view === "tray") renderTray();
});
await api.onUpdate((event: UpdateState) => {
  state.update = event;
  if (event.phase === "ready" || event.phase === "available") promptKind = "update";
  else if (event.phase === "current" || event.phase === "failed" || event.phase === "installing") promptKind = "update-result";
  if (view !== "logs") renderCurrent();
});
await api.onExitPrompt(() => {
  promptKind = "exit";
  renderCurrent();
});
await api.onCancelSetup(() => {
  promptKind = "cancel-setup";
  renderCurrent();
});
await api.onRefresh(async () => {
  state = await api.state();
  if (view !== "logs") renderCurrent();
});

window.addEventListener("beforeunload", () => {
  logSubscriptions.dispose();
});

renderCurrent();
