import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SettingsDialog } from "./SettingsDialog";
import userEvent from "@testing-library/user-event";

const mocks = vi.hoisted(() => ({
  listApiKeys: vi.fn(),
  listEmbeddingProfiles: vi.fn(),
  listenToEmbeddingEvents: vi.fn(),
}));

vi.mock("../api", () => ({
  api: {
    listApiKeys: mocks.listApiKeys,
    listEmbeddingProfiles: mocks.listEmbeddingProfiles,
  },
  isUnavailableCommandError: () => false,
  listenToEmbeddingEvents: mocks.listenToEmbeddingEvents,
}));

describe("SettingsDialog browser safety", () => {
  it("changes the application language, persists it and can return to credentials", async () => {
    const user = userEvent.setup();
    render(<SettingsDialog isNative={false} keyInputRef={createRef<HTMLInputElement>()} onClose={vi.fn()} />);
    const general = screen.getByRole("button", { name: "通用设置" });
    await user.click(general);
    expect(general).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("combobox", { name: "语言" }).closest("label")).toHaveClass("compact-select");
    await user.selectOptions(screen.getByLabelText("语言"), "en");
    expect(screen.getByRole("button", { name: "General" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByLabelText("Language")).toHaveValue("en");
    expect(localStorage.getItem("deadalus.language")).toBe("en");
    expect(document.documentElement.lang).toBe("en");
    await user.selectOptions(screen.getByLabelText("Language"), "zh");
    await user.click(screen.getByRole("button", { name: "apikeys管理" }));
    expect(screen.getByRole("heading", { name: "设置" })).toBeInTheDocument();
    expect(screen.getByPlaceholderText("桌面端可用")).toBeDisabled();
  });
  it("does not invoke native APIs and disables secret entry", () => {
    render(
      <SettingsDialog
        isNative={false}
        keyInputRef={createRef<HTMLInputElement>()}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByText("PREVIEW")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("桌面端可用")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "自适应" })).not.toBeInTheDocument();
    expect(
      screen.getByText(/浏览器预览不会调用 Credential Manager/),
    ).toBeInTheDocument();
    expect(mocks.listApiKeys).not.toHaveBeenCalled();
    expect(mocks.listEmbeddingProfiles).not.toHaveBeenCalled();
    expect(mocks.listenToEmbeddingEvents).not.toHaveBeenCalled();
  });
});
