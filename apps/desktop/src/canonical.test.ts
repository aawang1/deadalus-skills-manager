import { describe, expect, it } from "vitest";
import { filterCanonicalSkills } from "./canonical";
import type { InstalledSkill, ViewId } from "./types";

const skills: InstalledSkill[] = [
  {
    skillId: "shared",
    name: "Shared Skill",
    path: "shared",
    sourcePath: "shared",
    scope: "user",
    isBuiltIn: false,
    enabledAgents: ["cursor", "codex"],
    disabledAgents: [],
    inLibrary: true,
    backupSuppressed: false,
  },
  {
    skillId: "claude-only",
    name: "Claude Helper",
    path: "claude",
    sourcePath: "claude",
    scope: "user",
    isBuiltIn: false,
    enabledAgents: ["claude-code"],
    disabledAgents: [],
    inLibrary: false,
    backupSuppressed: false,
  },
];

describe("filterCanonicalSkills", () => {
  it.each<ViewId>(["all", "codex", "cursor", "claude-code", "custom:virtual", "custom:project"])(
    "hides built-ins in %s, including when searched, without changing the snapshot",
    (activeView) => {
      const normal: InstalledSkill = { ...skills[0], enabledAgents: ["codex", "cursor", "claude-code"] };
      const builtIn: InstalledSkill = { ...normal, skillId: "built-in", name: "Built-in Helper", isBuiltIn: true };
      const snapshot = [normal, builtIn];
      const options = { activeView, agentFilter: "all" as const, searchQuery: "", customSkillIds: new Set([normal.skillId, builtIn.skillId]) };
      expect(filterCanonicalSkills(snapshot, options)).toEqual([normal]);
      expect(filterCanonicalSkills(snapshot, { ...options, searchQuery: "Built-in" })).toEqual([]);
      expect(snapshot).toEqual([normal, builtIn]);
      expect(builtIn.isBuiltIn).toBe(true);
    },
  );
  it("filters an existing canonical snapshot by agent without rescanning", () => {
    const original = [...skills];
    const result = filterCanonicalSkills(skills, {
      activeView: "cursor",
      agentFilter: "all",
      searchQuery: "",
    });

    expect(result.map((skill) => skill.skillId)).toEqual(["shared"]);
    expect(skills).toEqual(original);
    expect(result[0]).toBe(skills[0]);
  });

  it("combines the all-view agent filter with text search", () => {
    expect(
      filterCanonicalSkills(skills, {
        activeView: "all",
        agentFilter: "claude-code",
        searchQuery: "helper",
      }).map((skill) => skill.skillId),
    ).toEqual(["claude-only"]);
  });
});
