import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { InstalledSkill } from "../types";
import { AgentComposer } from "./AgentComposer";

vi.mock("../api", () => ({ api: { recommendAgentSkills: vi.fn() } }));

const skill = {
  skillId: "skill-1", name: "Web UI", description: "Build interfaces", isBuiltIn: false,
  enabledAgents: [], disabledAgents: [], inLibrary: true, libraryPath: "D:/skills/web-ui",
} as unknown as InstalledSkill;

describe("Agent creation recommendation", () => {
  beforeEach(() => vi.mocked(api.recommendAgentSkills).mockReset());

  it("searches only the active visualized category and clears graph highlights on deletion", async () => {
    vi.mocked(api.recommendAgentSkills).mockResolvedValue({
      viewId: "custom:design", summary: "Web UI skills", warnings: ["候选分析第 2/2 组失败（1 个 Skills）：模型请求超时。"], results: [{ skillId: "skill-1", score: 0.91, parentScore: 0.9, expired: false, matchedTypes: [], evidence: [] }],
    });
    const change = vi.fn();
    const user = userEvent.setup();
    render(<AgentComposer windowId={2} activeView="custom:design" skills={[skill]} onRecommendationChange={change} />);
    await user.click(screen.getByRole("button", { name: "新建" }));
    await user.type(screen.getByRole("textbox", { name: "Agent 2 输入" }), "需要网页界面设计");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(api.recommendAgentSkills).toHaveBeenCalledWith("custom:design", "需要网页界面设计"));
    expect(await screen.findByText("Web UI skills")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("模型请求超时");
    expect(change).toHaveBeenCalledWith(2, "custom:design", ["skill-1"]);
    await user.click(screen.getByRole("button", { name: "删除推荐结果" }));
    await user.click(screen.getByRole("button", { name: "确认删除" }));
    expect(screen.queryByText("Web UI skills")).not.toBeInTheDocument();
    expect(change).toHaveBeenLastCalledWith(2, null, []);
  });
});
