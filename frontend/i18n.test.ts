import { describe, expect, it } from "vitest";
import { translate } from "./i18n";

describe("translations", () => {
  it("defaults to concise product language", () => {
    expect(translate("en-US", "openLogs")).toBe("Open logs");
    expect(translate("zh-CN", "openLogs")).toBe("打开日志");
  });

  it("substitutes update versions", () => {
    expect(translate("en-US", "updateReady", { version: "0.4.1" })).toBe("Update 0.4.1 is ready");
    expect(translate("en-US", "preparingUpdate", { version: "0.1.0-rc.8" })).toBe("Preparing DSH 0.1.0-rc.8…");
    expect(translate("zh-CN", "upToDate")).toBe("当前已是最新版本");
  });
});
