import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const mocks = vi.hoisted(() => ({
  getCanonicalSkillsSnapshot: vi.fn(),
  refreshCanonicalSkillsSnapshot: vi.fn(),
  semanticSearch: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("./api", () => ({
  isNativeRuntime: () => true,
  api: {
    getCanonicalSkillsSnapshot: mocks.getCanonicalSkillsSnapshot,
    refreshCanonicalSkillsSnapshot: mocks.refreshCanonicalSkillsSnapshot,
    semanticSearch: mocks.semanticSearch,
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

describe("B3 assistant platform", () => {
  beforeEach(() => {
    mocks.getCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-a",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.refreshCanonicalSkillsSnapshot.mockResolvedValue({
      snapshotId: "snapshot-b",
      searchedPaths: [],
      warnings: [],
      skills: [],
    });
    mocks.semanticSearch.mockResolvedValue([]);
    mocks.listen.mockResolvedValue(() => undefined);
  });

  it("starts collapsed, toggles from the circular control, and retains search state", async () => {
    const user = userEvent.setup();
    render(<App />);

    const platform = screen.getByRole("complementary", {
      name: "B3 语义搜索平台",
    });
    const expand = screen.getByRole("button", { name: "展开语义搜索" });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    expect(platform.firstElementChild).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByRole("main", { name: "A1 可视化工作区" })).toBeEmptyDOMElement();

    await user.click(expand);
    const collapse = screen.getByRole("button", { name: "收起语义搜索" });
    expect(collapse).toHaveAttribute("aria-expanded", "true");
    expect(platform.firstElementChild).toHaveAttribute("aria-hidden", "false");

    const searchbox = screen.getByRole("searchbox");
    await user.type(searchbox, "retained query");
    await user.click(collapse);
    expect(screen.getByRole("button", { name: "展开语义搜索" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    await user.click(screen.getByRole("button", { name: "展开语义搜索" }));
    expect(screen.getByRole("searchbox")).toHaveValue("retained query");
  });

  it("collapses B2 to an arrow strip while keeping the category selected", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "所有 Skills" }));
    const category = screen.getByRole("button", { name: "所有 Skills" });
    expect(category).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();

    const collapse = screen.getByRole("button", { name: "收起 Skills 列表" });
    expect(collapse).toHaveAttribute("aria-expanded", "true");
    await user.click(collapse);

    expect(category).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("heading", { name: "所有 Skills" })).not.toBeInTheDocument();
    const expand = screen.getByRole("button", { name: "展开 Skills 列表" });
    expect(expand).toHaveAttribute("aria-expanded", "false");

    await user.click(expand);
    expect(screen.getByRole("heading", { name: "所有 Skills" })).toBeInTheDocument();
  });
});
