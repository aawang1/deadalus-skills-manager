import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { DisplaySummary, translateSummary } from "./DisplaySummary";
import { setLanguage } from "../i18n";

const translate = vi.hoisted(() => vi.fn());
vi.mock("../api", () => ({ api: { translateDisplayTexts: translate } }));
afterEach(() => { setLanguage("zh"); Reflect.deleteProperty(window, "__TAURI_INTERNALS__"); });

it("batches requests, caches each target language separately and never overwrites the input", async () => {
  translate.mockImplementation(async (texts: string[], language: string) => texts.map(text => `${language}:${text}`));
  const source = "Unique summary";
  expect(await Promise.all([translateSummary(source, "en"), translateSummary(source, "en")])).toEqual([`en:${source}`, `en:${source}`]);
  expect(translate).toHaveBeenCalledTimes(1);
  await translateSummary(source, "en");
  expect(translate).toHaveBeenCalledTimes(1);
  expect(await translateSummary(source, "zh")).toBe(`zh:${source}`);
  expect(source).toBe("Unique summary");
});

it("ignores an old-language response after a language switch", async () => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
  let finish: (values: string[]) => void = () => {};
  translate.mockImplementationOnce(() => new Promise<string[]>(resolve => { finish = resolve; }))
    .mockResolvedValueOnce(["English summary"]);
  render(<DisplaySummary text="Delayed source"/>);
  await waitFor(() => expect(translate).toHaveBeenCalled());
  await act(async () => { setLanguage("en"); finish(["旧中文结果"]); });
  expect(await screen.findByText("English summary")).toBeInTheDocument();
  expect(screen.queryByText("旧中文结果")).not.toBeInTheDocument();
});
