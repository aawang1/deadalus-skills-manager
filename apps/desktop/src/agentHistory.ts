import type { AgentSkillRecommendation, InstalledSkill, ViewId } from "./types";

export interface AgentHistoryEntry {
  id: string;
  viewId: ViewId;
  request: string;
  createdAt: string;
  result?: AgentSkillRecommendation;
  error?: string;
  status?: "pending" | "completed" | "failed" | "interrupted";
  selectedIds: string[];
  skills: InstalledSkill[];
}
export const agentHistoryKey = (id: number) => `deadalus.agent-history.v1.${id}`;
export function loadAgentHistory(id: number): AgentHistoryEntry[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(agentHistoryKey(id)) ?? "[]");
    if (!Array.isArray(value)) return [];
    return value.filter((entry): entry is AgentHistoryEntry =>
      entry && typeof entry.id === "string" && typeof entry.viewId === "string" &&
      typeof entry.request === "string" && typeof entry.createdAt === "string" &&
      Array.isArray(entry.selectedIds) && entry.selectedIds.every((id: unknown) => typeof id === "string") &&
      Array.isArray(entry.skills) && entry.skills.every((skill: InstalledSkill) => skill && typeof skill.skillId === "string") &&
      (!entry.result || (Array.isArray(entry.result.results) && typeof entry.result.summary === "string")))
      .map((entry) => entry.status === "pending" ? { ...entry, status: "interrupted" as const } : entry);
  } catch { return []; }
}
export function removeAgentHistory(id: number) {
  localStorage.removeItem(agentHistoryKey(id));
}
