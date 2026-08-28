import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EmbeddingJob } from "../types";
import { IndexSyncPanel } from "./IndexSyncPanel";

const mocks = vi.hoisted(() => ({
  getIndexSyncStatus: vi.fn(),
  getIndexDiff: vi.fn(),
  listEmbeddingJobs: vi.fn(),
  deleteEmbeddingJobHistory: vi.fn(),
  clearEmbeddingJobHistory: vi.fn(),
  setIgnoreBuiltInSkills: vi.fn(),
  scanEmbeddingChanges: vi.fn(),
}));

vi.mock("../api", () => ({
  api: mocks,
  isUnavailableCommandError: () => false,
}));

const completedJob: EmbeddingJob = {
  jobId: "job-completed",
  profileId: "profile-a",
  kind: "incremental",
  status: "completed",
  totalItems: 2,
  completedItems: 2,
  estimatedTokens: 20,
  actualTokens: 18,
  createdAt: 1,
  updatedAt: 2,
};

const runningJob: EmbeddingJob = {
  ...completedJob,
  jobId: "job-running",
  status: "running",
  completedItems: 1,
};

describe("IndexSyncPanel Jobs history", () => {
  beforeEach(() => {
    mocks.getIndexSyncStatus.mockResolvedValue({ autoUpdate: false, ignoreBuiltInSkills: false, indexedSkills: 2, pendingChanges: 0 });
    mocks.getIndexDiff.mockResolvedValue({ added: 0, changed: 0, removed: 0, unchanged: 2 });
    mocks.listEmbeddingJobs.mockResolvedValue([completedJob, runningJob]);
    mocks.deleteEmbeddingJobHistory.mockResolvedValue(true);
    mocks.clearEmbeddingJobHistory.mockResolvedValue(1);
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  it("deletes one finished Job after confirmation but offers cancel for active Jobs", async () => {
    const user = userEvent.setup();
    render(<IndexSyncPanel isNative activeProfileId="profile-a" profiles={[]} onEstimate={vi.fn()} refreshVersion={0} notify={vi.fn()} />);

    await user.click(await screen.findByRole("button", { name: "删除 incremental Job 历史" }));
    expect(window.confirm).toHaveBeenCalledOnce();
    expect(mocks.deleteEmbeddingJobHistory).toHaveBeenCalledWith("job-completed");
    await waitFor(() => expect(screen.queryByRole("button", { name: "删除 incremental Job 历史" })).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "取消" })).toBeInTheDocument();
  });

  it("clears only finished history and reloads the server list", async () => {
    const user = userEvent.setup();
    mocks.listEmbeddingJobs.mockResolvedValueOnce([completedJob, runningJob]).mockResolvedValueOnce([runningJob]);
    render(<IndexSyncPanel isNative activeProfileId="profile-a" profiles={[]} onEstimate={vi.fn()} refreshVersion={0} notify={vi.fn()} />);

    await user.click(await screen.findByRole("button", { name: "清空历史" }));
    expect(mocks.clearEmbeddingJobHistory).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "取消" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "清空历史" })).toBeDisabled());
  });
});
