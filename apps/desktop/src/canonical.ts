import type { AgentId, InstalledSkill, ViewId } from "./types";

export function filterCanonicalSkills(
  skills: InstalledSkill[],
  options: {
    activeView: ViewId | null;
    agentFilter: "all" | AgentId;
    searchQuery: string;
    customSkillIds?: ReadonlySet<string>;
  },
): InstalledSkill[] {
  const query = options.searchQuery.trim().toLocaleLowerCase();
  const isCustomView = options.activeView?.startsWith("custom:") ?? false;
  const selectedAgent: "all" | AgentId = isAgentView(options.activeView)
    ? options.activeView
    : options.agentFilter;

  return skills.filter((skill) => {
    const matchesQuery =
      !query ||
      skill.name.toLocaleLowerCase().includes(query) ||
      skill.description?.toLocaleLowerCase().includes(query);
    const matchesAgent =
      selectedAgent === "all" || skill.enabledAgents.includes(selectedAgent);
    const matchesCustomCategory =
      !isCustomView || options.customSkillIds?.has(skill.skillId);
    return Boolean(matchesQuery && matchesAgent && matchesCustomCategory);
  });
}

function isAgentView(view: ViewId | null): view is AgentId {
  return view === "claude-code" || view === "cursor" || view === "codex";
}
