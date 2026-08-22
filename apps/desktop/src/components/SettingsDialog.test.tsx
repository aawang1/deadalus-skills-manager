import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SettingsDialog } from "./SettingsDialog";

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
    expect(
      screen.getByText(/浏览器预览不会调用 Credential Manager/),
    ).toBeInTheDocument();
    expect(mocks.listApiKeys).not.toHaveBeenCalled();
    expect(mocks.listEmbeddingProfiles).not.toHaveBeenCalled();
    expect(mocks.listenToEmbeddingEvents).not.toHaveBeenCalled();
  });
});
