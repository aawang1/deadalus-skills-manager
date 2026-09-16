import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateCategoryDialog } from "./CreateCategoryDialog";

const skills = [
  {
    skillId: "skill-web",
    name: "Web Design",
    description: "构建网页视觉系统",
    path: "skills/web-design",
    sourcePath: "skills",
    scope: "user" as const,
    isBuiltIn: false,
    enabledAgents: ["codex" as const],
    disabledAgents: [],
    inLibrary: true,
    backupSuppressed: false,
  },
  {
    skillId: "skill-game",
    name: "Game UI",
    description: "设计游戏界面",
    path: "skills/game-ui",
    sourcePath: "skills",
    scope: "user" as const,
    isBuiltIn: false,
    enabledAgents: ["cursor" as const],
    disabledAgents: [],
    inLibrary: true,
    backupSuppressed: false,
  },
  {
    skillId: "skill-built-in",
    name: "Built-in Helper",
    description: "Agent 自带 Skill",
    path: "skills/built-in-helper",
    sourcePath: "skills",
    scope: "system" as const,
    isBuiltIn: true,
    enabledAgents: ["codex" as const],
    disabledAgents: [],
    inLibrary: true,
    backupSuppressed: false,
  },
];

describe("CreateCategoryDialog", () => {
  it("requires complete metadata and a double-clicked Skill before creation", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(
      <CreateCategoryDialog
        skills={skills}
        onClose={vi.fn()}
        onCreate={onCreate}
      />,
    );

    const create = screen.getByRole("button", { name: "创建类别" });
    expect(create).toBeDisabled();
    await user.type(screen.getByLabelText("类别名称"), "视觉工具");
    await user.dblClick(screen.getByRole("button", { name: "选择信号蓝" }));
    await user.type(screen.getByLabelText("类别简介"), "集中管理视觉设计 Skills");
    await user.dblClick(screen.getByRole("button", { name: /Web Design/ }));
    expect(create).toBeEnabled();

    await user.click(create);
    expect(onCreate).toHaveBeenCalledWith({
      name: "视觉工具",
      color: "#4f8cff",
      description: "集中管理视觉设计 Skills",
      skillIds: ["skill-web"],
    });
  });

  it("filters the selectable Skills without clearing current selections", async () => {
    const user = userEvent.setup();
    render(
      <CreateCategoryDialog
        skills={skills}
        onClose={vi.fn()}
        onCreate={vi.fn()}
      />,
    );

    await user.dblClick(screen.getByRole("button", { name: /Game UI/ }));
    await user.type(screen.getByRole("searchbox", { name: "搜索可选 Skills" }), "Web");
    expect(screen.getByRole("button", { name: /Web Design/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Game UI/ })).not.toBeInTheDocument();
    expect(screen.getByText("1 已选择")).toBeInTheDocument();
  });

  it("never offers built-in Skills for custom category selection", () => {
    render(
      <CreateCategoryDialog
        skills={skills}
        onClose={vi.fn()}
        onCreate={vi.fn()}
      />,
    );

    expect(screen.queryByText("Built-in Helper")).not.toBeInTheDocument();
    expect(screen.getByText(/已忽略内置/)).toBeInTheDocument();
  });
});
