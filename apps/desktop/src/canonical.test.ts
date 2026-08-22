import { describe, expect, it } from "vitest";
import { filterCanonicalSkills } from "./canonical";
import type { InstalledSkill } from "./types";

const skills: InstalledSkill[] = [
  {
    skillId: "shared",
    name: "Shared Skill",
    path: "shared",
    sourcePath: "shared",
    scope: "user",
    isBuiltIn: false,
    enabledAgents: ["cursor", "codex"],
    inLibrary: true,
  },
  {
    skillId: "claude-only",
    name: "Claude Helper",
    path: "claude",
    sourcePath: "claude",
    scope: "user",
    isBuiltIn: false,
    enabledAgents: ["claude-code"],
    inLibrary: false,
  },
];

describe("filterCanonicalSkills", () => {
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
