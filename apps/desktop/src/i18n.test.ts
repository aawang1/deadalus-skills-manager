import { afterEach, describe, expect, it } from "vitest";
import { getLanguage, setLanguage, tr } from "./i18n";
import { english } from "./locales/en";

afterEach(() => setLanguage("zh"));
describe("display language", () => {
  it("defaults to Chinese and persists English without changing interpolated content", () => {
    localStorage.removeItem("deadalus.language");
    expect(getLanguage()).toBe("zh");
    expect(tr("将 {0} 复制到 {1} 吗？", "诗词", "Web")).toBe("将 诗词 复制到 Web 吗？");
    setLanguage("en");
    expect(tr("将 {0} 复制到 {1} 吗？", "诗词", "Web")).toBe("Copy 诗词 to Web?");
    expect(tr("通用设置")).toBe("General");
    expect(getLanguage()).toBe("en");
  });
  it("keeps every catalog interpolation intact", () => {
    for (const [source, target] of Object.entries(english)) {
      expect((target.match(/\{\d+\}/g) ?? []).sort(), source).toEqual((source.match(/\{\d+\}/g) ?? []).sort());
    }
  });
});
