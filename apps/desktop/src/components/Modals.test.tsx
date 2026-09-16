import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { PreflightEstimate, ProfileChangeRequest } from "../types";
import { PreflightEstimateModal } from "./PreflightEstimateModal";
import { ProfileSwitchModal } from "./ProfileSwitchModal";

const request: ProfileChangeRequest = {
  provider: "openai",
  reason: "profile",
  sourceProfileId: "old",
  targetProfileId: "new",
};

const estimate: PreflightEstimate = {
  complexityLevel: 2,
  skillCount: 2,
  fileCount: 3,
  analysisFileCount: 2,
  embeddingFileCount: 2,
  embeddableTextCount: 4,
  parentCount: 5,
  chunkCountLow: 6,
  chunkCountHigh: 8,
  analysisTokensLow: 10,
  analysisTokensHigh: 12,
  embeddingTokensLow: 20,
  embeddingTokensHigh: 30,
  estimatedSecondsLow: 12,
  estimatedSecondsHigh: 90,
  provider: "openai",
  model: "text-embedding-3-small",
  estimatedAt: 1,
  confidence: "high",
  missingReasons: [],
};

describe("ProfileSwitchModal", () => {
  it("reports cancel and both isolated-build choices", () => {
    const onCancel = vi.fn();
    const onGlobalUpdate = vi.fn();
    const onRecommendedMigration = vi.fn();
    render(
      <ProfileSwitchModal
        request={request}
        onCancel={onCancel}
        onGlobalUpdate={onGlobalUpdate}
        onRecommendedMigration={onRecommendedMigration}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "全局更新" }));
    fireEvent.click(screen.getByRole("button", { name: "推荐移植" }));
    fireEvent.click(
      screen.getAllByRole("button", { name: "取消模型改变" })[1],
    );

    expect(onGlobalUpdate).toHaveBeenCalledOnce();
    expect(onRecommendedMigration).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

describe("PreflightEstimateModal", () => {
  it("confirms or closes without implicit execution", () => {
    const onConfirm = vi.fn();
    const onClose = vi.fn();
    render(
      <PreflightEstimateModal
        estimate={estimate}
        onConfirm={onConfirm}
        onClose={onClose}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "确认执行" }));
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));

    expect(onConfirm).toHaveBeenCalledOnce();
    expect(onClose).toHaveBeenCalledOnce();
  });
});
