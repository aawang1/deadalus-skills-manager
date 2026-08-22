import type { AgentId, InstalledSkill, ViewId } from "./types";

export function filterCanonicalSkills(
  skills: InstalledSkill[],
  options: {
    activeView: ViewId | null;
    agentFilter: "all" | AgentId;
    searchQuery: string;
  },
): InstalledSkill[] {
  const query = options.searchQuery.trim().toLocaleLowerCase();
  const selectedAgent =
    options.activeView && options.activeView !== "all"
      ? options.activeView
      : options.agentFilter;

  return skills.filter((skill) => {
    const matchesQuery =
      !query ||
      skill.name.toLocaleLowerCase().includes(query) ||
      skill.description?.toLocaleLowerCase().includes(query);
    const matchesAgent =
      selectedAgent === "all" || skill.enabledAgents.includes(selectedAgent);
    return Boolean(matchesQuery && matchesAgent);
  });
}
