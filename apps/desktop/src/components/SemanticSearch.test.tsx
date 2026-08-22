import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstalledSkill, SemanticSearchResult } from "../types";
import { SemanticSearch } from "./SemanticSearch";

const mocks = vi.hoisted(() => ({
  semanticSearch: vi.fn(),
}));

vi.mock("../api", () => ({
  api: {
    semanticSearch: mocks.semanticSearch,
  },
}));

const skills: InstalledSkill[] = [
  {
    skillId: "skill-a",
    name: "Release Assistant",
    path: "a",
    sourcePath: "a",
    scope: "user",
    isBuiltIn: false,
    enabledAgents: ["cursor"],
    inLibrary: true,
  },
];

describe("SemanticSearch", () => {
  beforeEach(() => mocks.semanticSearch.mockReset());

  it("renders the zero-result state", async () => {
    mocks.semanticSearch.mockResolvedValue([]);
    const user = userEvent.setup();
    render(<SemanticSearch isNative skills={skills} />);

    await user.type(screen.getByRole("searchbox"), "publish a release");
    await user.click(screen.getByRole("button", { name: "搜索" }));

    expect(await screen.findByText("没有匹配结果")).toBeInTheDocument();
  });

  it("renders skill names, reasons, evidence, and expired state", async () => {
    const result: SemanticSearchResult = {
      skillId: "skill-a",
      score: 0.91,
      parentScore: 0.9,
      chunkScore: 0.92,
      expired: true,
      matchedTypes: ["workflow"],
      evidence: [
        {
          embeddingId: "embedding-a",
          vectorType: "workflow",
          level: "chunk",
          rawScore: 0.92,
          expired: true,
          headingPath: "Deploy",
        },
      ],
    };
    mocks.semanticSearch.mockResolvedValue([result]);
    const user = userEvent.setup();
    render(<SemanticSearch isNative skills={skills} />);

    await user.type(screen.getByRole("searchbox"), "release");
    await user.click(screen.getByRole("button", { name: "搜索" }));

    const title = await screen.findByText("Release Assistant");
    const resultCard = title.closest("li");
    expect(resultCard).not.toBeNull();
    expect(screen.getByText("索引已过期")).toBeInTheDocument();
    expect(within(resultCard!).getByText("工作流")).toBeInTheDocument();
    await user.click(screen.getByText("匹配证据 · 1"));
    expect(screen.getByText(/Deploy · 已过期/)).toBeInTheDocument();
  });

  it("requires confirmation before clearing retained search history", async () => {
    mocks.semanticSearch.mockResolvedValue([]);
    const confirm = vi
      .spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    const user = userEvent.setup();
    render(<SemanticSearch isNative skills={skills} />);

    const searchbox = screen.getByRole("searchbox");
    await user.type(searchbox, "publish a release");
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Agent 筛选" }),
      "cursor",
    );
    await user.click(screen.getByRole("button", { name: "搜索" }));
    expect(await screen.findByText("没有匹配结果")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "清空历史" }));
    expect(searchbox).toHaveValue("publish a release");
    expect(
      screen.getByRole("combobox", { name: "Agent 筛选" }),
    ).toHaveValue("cursor");

    await user.click(screen.getByRole("button", { name: "清空历史" }));
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(searchbox).toHaveValue("");
    expect(
      screen.getByRole("combobox", { name: "Agent 筛选" }),
    ).toHaveValue("all");
    expect(screen.getByText("输入任务描述开始搜索")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "清空历史" })).toBeDisabled();
  });
});
