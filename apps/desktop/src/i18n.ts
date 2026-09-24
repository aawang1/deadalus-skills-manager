import { useEffect, useSyncExternalStore } from "react";
import { english } from "./locales/en";

export type Language = "zh" | "en";
const storageKey = "deadalus.language";
const listeners = new Set<() => void>();
export function getLanguage(): Language {
  try { return localStorage.getItem(storageKey) === "en" ? "en" : "zh"; }
  catch { return "zh"; }
}
export function setLanguage(language: Language) {
  localStorage.setItem(storageKey, language);
  document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  listeners.forEach((listener) => listener());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  const onStorage = (event: StorageEvent) => { if (event.key === storageKey) listener(); };
  window.addEventListener("storage", onStorage);
  return () => { listeners.delete(listener); window.removeEventListener("storage", onStorage); };
}
export function useLanguage() {
  const language = useSyncExternalStore(subscribe, getLanguage, () => "zh" as Language);
  useEffect(() => { document.documentElement.lang = language === "zh" ? "zh-CN" : "en"; }, [language]);
  return language;
}
export function tr(source: string, ...values: unknown[]): string {
  const template = getLanguage() === "en" ? english[source] ?? source : source;
  return template.replace(/\{(\d+)\}/g, (_, index: string) => String(values[Number(index)] ?? `{${index}}`));
}
export function displayStatus(value: string): string {
  const names: Record<string, string> = {
    ready: "就绪", draft: "草稿", building: "构建中", failed: "失败", completed: "已完成",
    running: "运行中", pending: "待处理", queued: "排队中", cancelled: "已取消", canceled: "已取消",
    paused: "已暂停", expired: "已过期", stale: "已过期", high: "高", medium: "中", low: "低",
    full_rebuild: "全量重建", incremental: "增量更新", profile_migration: "配置迁移",
  };
  return names[value] ? tr(names[value]) : value;
}
