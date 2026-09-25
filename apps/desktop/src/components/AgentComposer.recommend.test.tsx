import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { AgentSkillRecommendation, InstalledSkill } from "../types";
import { AgentComposer } from "./AgentComposer";

vi.mock("../api", () => ({ api: { recommendAgentSkills: vi.fn() } }));

const skill = {
  skillId: "skill-1", name: "Web UI", description: "Build interfaces", isBuiltIn: false,
  enabledAgents: [], disabledAgents: [], inLibrary: true, libraryPath: "D:/skills/web-ui",
} as unknown as InstalledSkill;

describe("Agent creation recommendation", () => {
  beforeEach(() => { vi.mocked(api.recommendAgentSkills).mockReset(); localStorage.clear(); });

  it("clears the input immediately and replaces the pending bubble without clearing the next draft", async () => {
    let finish!: (result: AgentSkillRecommendation) => void;
    vi.mocked(api.recommendAgentSkills).mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const user = userEvent.setup();
    render(<AgentComposer windowId={11} activeView="all" skills={[skill]} />);
    await user.click(screen.getByRole("button", { name: "新建" }));
    const input = screen.getByRole("textbox", { name: "Agent 11 输入" });
    await user.type(input, "Analyze this");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(input).toHaveValue("");
    expect(screen.getByText("Analyze this")).toBeInTheDocument();
    const bubble = screen.getByRole("status", { name: "正在分析需求" });
    expect(bubble).toHaveTextContent("分析中......");
    expect(bubble.querySelector(".agent-history__dots")).toHaveAttribute("aria-hidden", "true");
    await user.type(input, "Next draft");
    await act(async () => finish({
      viewId: "all", summary: "Completed analysis", warnings: [],
      results: [{ skillId: "skill-1", score: 0.9, parentScore: 0.9, expired: false, matchedTypes: [], evidence: [] }],
    }));
    expect(screen.queryByRole("status", { name: "正在分析需求" })).not.toBeInTheDocument();
    expect(screen.getByText("Completed analysis")).toBeInTheDocument();
    expect(screen.getAllByText("Analyze this")).toHaveLength(1);
    expect(input).toHaveValue("Next draft");
  });

  it("marks pending saved history as interrupted on restart", () => {
    localStorage.setItem("deadalus.agent-history.v1.12", JSON.stringify([{
      id: "pending-entry", viewId: "all", request: "Pending request", createdAt: new Date().toISOString(),
      status: "pending", selectedIds: [], skills: [],
    }]));
    render(<AgentComposer windowId={12} activeView="all" />);
    expect(screen.getByText("上次分析已中断，请重新发送需求。")).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "正在分析需求" })).not.toBeInTheDocument();
  });

  it("appends requests, keeps history across refresh and remount, and isolates windows", async () => {
    vi.mocked(api.recommendAgentSkills).mockResolvedValue({
      viewId: "all", summary: "Saved result", warnings: [],
      results: [{ skillId: "skill-1", score: 0.91, parentScore: 0.9, expired: false, matchedTypes: [], evidence: [] }],
    });
    const user = userEvent.setup();
    const change = vi.fn();
    const { rerender, unmount } = render(<AgentComposer windowId={7} activeView="all" skills={[skill]} onRecommendationChange={change} />);
    await user.click(screen.getByRole("button", { name: "新建" }));
    for (const request of ["First request", "Second request"]) {
      await user.type(screen.getByRole("textbox", { name: "Agent 7 输入" }), request);
      await user.click(screen.getByRole("button", { name: "发送" }));
      await screen.findByText(request);
      await waitFor(() => expect(screen.getByRole("textbox", { name: "Agent 7 输入" })).toHaveValue(""));
    }
    expect(screen.getAllByText("Saved result")).toHaveLength(2);
    rerender(<AgentComposer windowId={7} activeView="all" skills={[skill]} indexRevision={5} onRecommendationChange={change} />);
    expect(screen.getByText("First request")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看并调整这次结果" }));
    await user.click(screen.getByRole("button", { name: "取消选中 Web UI" }));
    unmount();
    const restored = render(<AgentComposer windowId={7} activeView="all" skills={[skill]} onRecommendationChange={change} />);
    expect(screen.getByText("First request")).toBeInTheDocument();
    expect(screen.getByText("Second request")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看并调整这次结果" }));
    expect(screen.getByRole("button", { name: "创建新类别" })).toBeDisabled();
    restored.unmount();
    render(<AgentComposer windowId={8} activeView="all" skills={[skill]} />);
    expect(screen.queryByText("First request")).not.toBeInTheDocument();
  });

  it("keeps a failed request in history without erasing the previous result", async () => {
    vi.mocked(api.recommendAgentSkills).mockRejectedValue(new Error("network test failure"));
    const user = userEvent.setup();
    const { unmount } = render(<AgentComposer windowId={9} activeView="all" />);
    await user.click(screen.getByRole("button", { name: "新建" }));
    await user.type(screen.getByRole("textbox", { name: "Agent 9 输入" }), "Failed request");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await screen.findByText("Failed request");
    unmount();
    render(<AgentComposer windowId={9} activeView="all" />);
    expect(screen.getByText("Failed request")).toBeInTheDocument();
    expect(screen.getByText(/network test failure/)).toBeInTheDocument();
  });

  it("reports local storage failure without preventing use of the window", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
    render(<AgentComposer windowId={10} activeView="all" />);
    expect(screen.getByRole("alert")).toHaveTextContent("历史记录保存失败");
    expect(screen.getByRole("button", { name: "发送" })).toBeEnabled();
  });

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
