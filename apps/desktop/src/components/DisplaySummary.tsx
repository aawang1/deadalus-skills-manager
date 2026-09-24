import { useEffect, useState } from "react";
import { api } from "../api";
import { tr, useLanguage, type Language } from "../i18n";

const cacheKey = "deadalus.display-translations.v1";
const cache = new Map<string, string>();
try {
  const saved: unknown = JSON.parse(localStorage.getItem(cacheKey) || "[]");
  if (Array.isArray(saved)) for (const pair of saved.slice(-500)) {
    if (Array.isArray(pair) && typeof pair[0] === "string" && typeof pair[1] === "string") cache.set(pair[0], pair[1]);
  }
} catch { /* Corrupt or unavailable cache never prevents rendering. */ }
type Pending = { text: string; language: Language; resolve: (s: string) => void; reject: (e: unknown) => void };
const requests = new Map<string, Promise<string>>();
const queue: Pending[] = [];
let running = false;
async function drain() {
  if (running) return;
  running = true;
  try {
    while (queue.length) {
      const batch = [queue.shift()!];
      let size = batch[0].text.length;
      while (queue.length && batch.length < 8 && queue[0].language === batch[0].language && size + queue[0].text.length <= 18000) {
        const next = queue.shift()!; size += next.text.length; batch.push(next);
      }
      try {
        const result = await api.translateDisplayTexts(batch.map(item => item.text), batch[0].language);
        if (result.length !== batch.length || result.some(text => !text?.trim())) throw new Error("Invalid translations");
        batch.forEach((item, index) => {
          cache.set(JSON.stringify([item.language, item.text]), result[index]);
          item.resolve(result[index]);
        });
        while (cache.size > 500) cache.delete(cache.keys().next().value!);
        try { localStorage.setItem(cacheKey, JSON.stringify([...cache])); } catch { /* Memory cache still works. */ }
      } catch (error) { batch.forEach(item => item.reject(error)); }
      finally { batch.forEach(item => requests.delete(JSON.stringify([item.language, item.text]))); }
    }
  } finally { running = false; }
}
export function translateSummary(text: string, language: Language): Promise<string> {
  const key = JSON.stringify([language, text]);
  if (cache.has(key)) return Promise.resolve(cache.get(key)!);
  if (requests.has(key)) return requests.get(key)!;
  if (text.length > 18000) return Promise.reject(new Error("Description is too long"));
  const promise = new Promise<string>((resolve, reject) => queue.push({ text, language, resolve, reject }));
  requests.set(key, promise);
  queueMicrotask(() => { void drain(); });
  return promise;
}

export function DisplaySummary({ text }: { text?: string }) {
  const language = useLanguage();
  const [state, setState] = useState<{ text: string; language: Language; value?: string; failed?: boolean }>();
  useEffect(() => {
    if (!text || !("__TAURI_INTERNALS__" in window)) return;
    let current = true;
    translateSummary(text, language).then(
      value => { if (current) setState({ text, language, value }); },
      () => { if (current) setState({ text, language, failed: true }); },
    );
    return () => { current = false; };
  }, [text, language]);
  const current = state?.text === text && state?.language === language ? state : undefined;
  return <span>{current?.value ?? text ?? tr("暂无简介")}{current?.failed && <small title={tr("概述翻译失败，保留原文；请检查 Agent 凭据后重试。")}> · {tr("原文")}</small>}</span>;
}
