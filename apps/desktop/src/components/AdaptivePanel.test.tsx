import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AdaptiveHistoryRecord,
  EmbeddingProfile,
  EmbeddingProfileSettings,
} from "../types";
import { AdaptivePanel } from "./AdaptivePanel";

const mocks = vi.hoisted(() => ({
  getEmbeddingProfileSettings: vi.fn(),
  listEmbeddingAdaptiveHistory: vi.fn(),
  listLocalValidationSamples: vi.fn(),
  listLocalValidationRuns: vi.fn(),
  setLocalAdaptiveEnabled: vi.fn(),
  proposeAdaptiveSettings: vi.fn(),
  applyAdaptiveHistory: vi.fn(),
  rejectAdaptiveHistory: vi.fn(),
  revertAdaptiveHistory: vi.fn(),
  exportLocalValidationData: vi.fn(),
  resetLocalValidationData: vi.fn(),
}));

vi.mock("../api", () => ({ api: mocks }));

const profile: EmbeddingProfile = {
  profileId: "profile-a",
  provider: "openai",
  model: "text-embedding-3-small",
  modelVersion: "1",
  dimensions: 1536,
  inputSchemaVersion: "input-v1",
  chunkPolicyVersion: "chunk-v1",
  credentialId: "credential-a",
  status: "active",
  isActive: true,
  createdAt: 1,
};

const settings: EmbeddingProfileSettings = {
  profile,
  defaults: {
    provider: "openai",
    model: "text-embedding-3-small",
    version: "1",
    dimensions: 1536,
  },
  credentialReady: true,
  adaptivePolicy: {
    profileId: "profile-a",
    h: 10000,
    m: 1000,
    u: 9000,
    l: 8000,
    localUpdateEnabled: false,
    policyVersion: "adaptive-v1",
    limitSource: "provider",
    limitSourceVersion: "limits-v1",
    effectiveAfter: 1,
    updatedAt: 1,
  },
  adaptiveHistory: [],
};

const proposed: AdaptiveHistoryRecord = {
  historyId: "proposal-a",
  profileId: "profile-a",
  parameterVersion: "adaptive-v2",
  decision: "proposed",
  createdAt: 2,
  proposal: {
    policy: {
      ...settings.adaptivePolicy,
      policyVersion: "adaptive-v2",
      m: 1200,
      u: 8800,
      l: 7600,
    },
    requiresReview: true,
    largeChange: false,
    effectiveFor: "future_jobs",
    costDiagnostic: {},
    costVeto: false,
    confidence: "medium",
    warnings: [],
  },
  evidence: {},
};

function renderPanel() {
  return render(
    <AdaptivePanel
      isNative
      profiles={[profile]}
      notify={vi.fn()}
    />,
  );
}

describe("AdaptivePanel", () => {
  beforeEach(() => {
    Object.values(mocks).forEach((mock) => mock.mockReset());
    mocks.getEmbeddingProfileSettings.mockResolvedValue(settings);
    mocks.listEmbeddingAdaptiveHistory.mockResolvedValue([]);
    mocks.listLocalValidationSamples.mockResolvedValue([]);
    mocks.listLocalValidationRuns.mockResolvedValue([]);
    mocks.setLocalAdaptiveEnabled.mockResolvedValue({
      ...settings.adaptivePolicy,
      localUpdateEnabled: true,
    });
    mocks.proposeAdaptiveSettings.mockResolvedValue(proposed);
  });

  it("toggles local adaptive updates and generates proposals explicitly", async () => {
    const user = userEvent.setup();
    renderPanel();

    const toggle = await screen.findByRole("switch", {
      name: "允许本地自适应更新",
    });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    await user.click(toggle);
    expect(mocks.setLocalAdaptiveEnabled).toHaveBeenCalledWith(
      "profile-a",
      true,
    );

    await user.click(screen.getByRole("button", { name: "生成自适应提案" }));
    expect(mocks.proposeAdaptiveSettings).toHaveBeenCalledWith("profile-a");
  });

  it("requires confirmation for proposal decisions and current-policy revert", async () => {
    const applied = {
      ...proposed,
      historyId: "applied-a",
      decision: "applied",
      appliedAt: 3,
    };
    mocks.getEmbeddingProfileSettings.mockResolvedValue({
      ...settings,
      adaptivePolicy: {
        ...settings.adaptivePolicy,
        appliedHistoryId: "applied-a",
      },
    });
    mocks.listEmbeddingAdaptiveHistory.mockResolvedValue([proposed, applied]);
    mocks.applyAdaptiveHistory.mockResolvedValue(settings.adaptivePolicy);
    mocks.rejectAdaptiveHistory.mockResolvedValue({
      ...proposed,
      decision: "rejected",
    });
    mocks.revertAdaptiveHistory.mockResolvedValue(settings.adaptivePolicy);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "应用提案" }),
    );
    expect(mocks.applyAdaptiveHistory).not.toHaveBeenCalled();
    confirm.mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "应用提案" }));
    expect(mocks.applyAdaptiveHistory).toHaveBeenCalledWith("proposal-a");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "拒绝提案" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "拒绝提案" }));
    expect(mocks.rejectAdaptiveHistory).toHaveBeenCalledWith("proposal-a");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "回滚此设置" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "回滚此设置" }));
    expect(mocks.revertAdaptiveHistory).toHaveBeenCalledWith("applied-a");
    expect(window.confirm).toHaveBeenCalledTimes(4);
  });

  it("exports only on click and protects reset with typed confirmation", async () => {
    const createObjectURL = vi.fn(() => "blob:adaptive-export");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => undefined);
    mocks.exportLocalValidationData.mockResolvedValue({
      schemaVersion: "local-v1",
      localOnly: true,
      externalWritePerformed: false,
      profileId: "profile-a",
      exportedAt: 1,
      samples: [],
      runs: [],
      feedbackEvents: [],
    });
    mocks.resetLocalValidationData.mockResolvedValue({
      samples: 2,
      runs: 1,
      feedbackEvents: 1,
      relations: 0,
    });
    const prompt = vi.spyOn(window, "prompt").mockReturnValue(null);
    const user = userEvent.setup();
    renderPanel();

    expect(mocks.exportLocalValidationData).not.toHaveBeenCalled();
    await user.click(await screen.findByRole("button", { name: "导出 JSON" }));
    expect(mocks.exportLocalValidationData).toHaveBeenCalledWith("profile-a");
    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(click).toHaveBeenCalledOnce();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:adaptive-export");

    await user.click(screen.getByRole("button", { name: "重置本地数据" }));
    expect(mocks.resetLocalValidationData).not.toHaveBeenCalled();
    prompt.mockReturnValue("RESET");
    await user.click(screen.getByRole("button", { name: "重置本地数据" }));
    expect(mocks.resetLocalValidationData).toHaveBeenCalledWith("profile-a");
  });
});
