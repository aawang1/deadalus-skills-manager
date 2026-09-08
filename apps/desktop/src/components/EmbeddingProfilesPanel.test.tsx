import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EmbeddingProfile } from "../types";
import { EmbeddingProfilesPanel } from "./EmbeddingProfilesPanel";

const mocks = vi.hoisted(() => ({
  deleteEmbeddingProfile: vi.fn(),
}));

vi.mock("../api", () => ({ api: mocks }));

const inactiveProfile: EmbeddingProfile = {
  profileId: "profile-ready",
  provider: "openai",
  model: "text-embedding-3-small",
  modelVersion: "1",
  dimensions: 1536,
  inputSchemaVersion: "input-v1",
  chunkPolicyVersion: "chunk-v1",
  credentialId: "credential-a",
  status: "ready",
  isActive: false,
  createdAt: 1,
};

const activeProfile: EmbeddingProfile = {
  ...inactiveProfile,
  profileId: "profile-active",
  status: "active",
  isActive: true,
};

function renderPanel(profiles = [inactiveProfile]) {
  const setProfiles = vi.fn();
  const notify = vi.fn();
  render(
    <EmbeddingProfilesPanel
      isNative
      keys={[]}
      profiles={profiles}
      setKeys={vi.fn()}
      setProfiles={setProfiles}
      notify={notify}
      requestProfileSwitch={vi.fn()}
    />,
  );
  return { setProfiles, notify };
}

describe("EmbeddingProfilesPanel profile deletion", () => {
  beforeEach(() => {
    mocks.deleteEmbeddingProfile.mockReset();
    mocks.deleteEmbeddingProfile.mockResolvedValue(true);
  });

  it("requires an in-app second confirmation before deleting", async () => {
    const user = userEvent.setup();
    const { setProfiles, notify } = renderPanel();

    await user.click(
      screen.getByRole("button", {
        name: "删除 text-embedding-3-small Profile",
      }),
    );
    expect(mocks.deleteEmbeddingProfile).not.toHaveBeenCalled();
    expect(
      screen.getByRole("alertdialog", {
        name: "确认删除 Embedding Profile",
      }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "确认删除" }));
    expect(mocks.deleteEmbeddingProfile).toHaveBeenCalledWith("profile-ready");
    await waitFor(() =>
      expect(
        screen.queryByRole("alertdialog", {
          name: "确认删除 Embedding Profile",
        }),
      ).not.toBeInTheDocument(),
    );
    expect(setProfiles).toHaveBeenCalledWith([]);
    expect(notify).toHaveBeenCalledWith("success", "Embedding Profile 已删除。");
  });

  it("cancels without deleting and protects the active profile", async () => {
    const user = userEvent.setup();
    renderPanel([inactiveProfile, activeProfile]);

    const deleteButtons = screen.getAllByRole("button", {
      name: "删除 text-embedding-3-small Profile",
    });
    await user.click(deleteButtons[0]);
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(mocks.deleteEmbeddingProfile).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();

    expect(deleteButtons[1]).toBeDisabled();
  });

  it("keeps the dialog open and displays a recoverable backend error", async () => {
    mocks.deleteEmbeddingProfile.mockRejectedValue(
      "该 Profile 仍有进行中的 Job；请先取消或等待任务结束。",
    );
    const user = userEvent.setup();
    const { notify } = renderPanel();

    await user.click(
      screen.getByRole("button", {
        name: "删除 text-embedding-3-small Profile",
      }),
    );
    await user.click(screen.getByRole("button", { name: "确认删除" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "该 Profile 仍有进行中的 Job；请先取消或等待任务结束。",
    );
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(notify).toHaveBeenCalledWith(
      "error",
      "该 Profile 仍有进行中的 Job；请先取消或等待任务结束。",
    );
  });
});
