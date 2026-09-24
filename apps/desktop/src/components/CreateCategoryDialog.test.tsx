import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CreateCategoryDialog } from "./CreateCategoryDialog";
import { api } from "../api";

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
  it("creates a project from a native folder selection without selecting Skills", async () => {
    const user = userEvent.setup();
    const pick = vi.spyOn(api, "selectProjectDirectory").mockResolvedValue("D:\\example");
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(<CreateCategoryDialog project skills={skills} onClose={vi.fn()} onCreate={onCreate} />);
    expect(screen.queryByRole("button", { name: /Web Design/ })).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("类别名称"), "项目测试");
    await user.dblClick(screen.getByRole("button", { name: "选择信号蓝" }));
    await user.type(screen.getByLabelText("类别简介"), "项目专用工具");
    expect(screen.getByRole("button", { name: "创建类别" })).toBeDisabled();
    const directoryButton = screen.getByRole("button", { name: "选择项目根目录文件夹" });
    expect(directoryButton).toHaveClass("project-directory-picker__button");
    expect(directoryButton).toHaveAccessibleDescription(/\.agents\/skills 与 \.claude\/skills/);
    expect(screen.getByText(/创建和复制时同步补齐项目/)).toHaveClass("credential-hint");
    await user.click(directoryButton);
    expect(screen.getByRole("status")).toHaveTextContent("D:\\example");
    await user.click(screen.getByRole("button", { name: "创建类别" }));
    expect(onCreate).toHaveBeenCalledWith(expect.objectContaining({ projectRoot: "D:\\example", skillIds: [] }));
    pick.mockRestore();
  });
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

  it("portals the Agent recommendation dialog outside the clipped B3 container", async () => {
    const user = userEvent.setup();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <div className="assistant-platform__content">
        <CreateCategoryDialog
          skills={skills}
          fixedSkillIds={["skill-web"]}
          onClose={vi.fn()}
          onCreate={onCreate}
        />
      </div>,
    );

    const dialog = screen.getByRole("dialog", { name: "新建自定义类别" });
    const backdrop = dialog.parentElement!;
    expect(container).not.toContainElement(dialog);
    expect(backdrop.parentElement).toBe(document.body);

    const create = screen.getByRole("button", { name: "创建类别" });
    expect(create).toBeVisible();
    await user.type(screen.getByLabelText("类别名称"), "诗词网站");
    await user.dblClick(screen.getByRole("button", { name: "选择信号蓝" }));
    await user.type(screen.getByLabelText("类别简介"), "Agent 推荐的诗词网站 Skills");
    expect(create).toBeEnabled();
    await user.click(create);
    expect(onCreate).toHaveBeenCalledWith({
      name: "诗词网站",
      color: "#4f8cff",
      description: "Agent 推荐的诗词网站 Skills",
      skillIds: ["skill-web"],
    });
  });
});
