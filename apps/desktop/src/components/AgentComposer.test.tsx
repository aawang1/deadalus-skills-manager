import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { AgentComposer } from "./AgentComposer";

vi.mock("../api", () => ({ api: { recommendAgentSkills: vi.fn() } }));

describe("AgentComposer", () => {
  it("toggles the new-category gold light without creating a category", async () => {
    const user = userEvent.setup();
    render(<AgentComposer windowId={1} />);
    const newButton = screen.getByRole("button", { name: "新建" });

    expect(newButton).toHaveAttribute("aria-pressed", "false");
    expect(newButton).not.toHaveClass("agent-composer__new--active");
    await user.click(newButton);
    expect(newButton).toHaveAttribute("aria-pressed", "true");
    expect(newButton).toHaveClass("agent-composer__new--active");
    await user.click(newButton);
    expect(newButton).toHaveAttribute("aria-pressed", "false");
    expect(newButton).not.toHaveClass("agent-composer__new--active");
  });

  it("shows the automatic category hint only after a one-second hover", async () => {
    render(<AgentComposer windowId={1} />);
    const newButton = screen.getByRole("button", { name: "新建" });
    const hoverArea = newButton.parentElement!;

    fireEvent.mouseEnter(hoverArea);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    await waitFor(
      () =>
        expect(screen.getByRole("tooltip")).toHaveTextContent(
          "自动新建skills类别",
        ),
      { timeout: 1600 },
    );
    fireEvent.mouseLeave(hoverArea);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("keeps send clickable and explains each missing prerequisite", async () => {
    const user = userEvent.setup();
    render(<AgentComposer windowId={1} />);

    const send = screen.getByRole("button", { name: "发送" });
    expect(send).toBeEnabled();
    await user.click(send);
    expect(screen.getByRole("alert")).toHaveTextContent("请先点击“新建”");

    await user.click(screen.getByRole("button", { name: "新建" }));
    await user.click(send);
    expect(screen.getByRole("alert")).toHaveTextContent("请先在左侧选择一个 Skills 类别");
    expect(api.recommendAgentSkills).not.toHaveBeenCalled();
  });
});
