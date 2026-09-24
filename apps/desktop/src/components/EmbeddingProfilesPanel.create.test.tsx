import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { EmbeddingProfilesPanel } from "./EmbeddingProfilesPanel";
import type { ApiKeyMetadata, EmbeddingProfile } from "../types";
const mocks = vi.hoisted(() => ({ getEmbeddingProfileDefaults: vi.fn(), createEmbeddingProfile: vi.fn() }));
vi.mock("../api", () => ({ api: mocks }));
const key: ApiKeyMetadata = { id: "qwen-key", provider: "qwen", purpose: "embedding", maskedKey: "masked", savedAt: 1, isAgentActive: false, isEmbeddingActive: true };
const props = { isNative: true, keys: [key], profiles: [] as EmbeddingProfile[], setKeys: vi.fn(), setProfiles: vi.fn(), notify: vi.fn(), requestProfileSwitch: vi.fn() };
beforeEach(() => {
  vi.clearAllMocks();
  mocks.getEmbeddingProfileDefaults.mockResolvedValue({ provider: "qwen", model: "text-embedding-v4", version: "1", dimensions: 1024 });
});

it("accepts only name and description and binds the selected embedding credential", async () => {
  const user = userEvent.setup();
  mocks.createEmbeddingProfile.mockResolvedValue({ profileId: "created", name: "My index", description: "My skills" });
  render(<EmbeddingProfilesPanel {...props} />);
  await screen.findByLabelText("名称");
  expect(screen.queryByLabelText("服务商")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("模型")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("版本")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("维度")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "创建 Profile" })).toBeDisabled();
  await user.type(screen.getByLabelText("名称"), "My index");
  await user.type(screen.getByLabelText("简介"), "My skills");
  await user.click(screen.getByRole("button", { name: "创建 Profile" }));
  expect(mocks.createEmbeddingProfile).toHaveBeenCalledWith({ name: "My index", description: "My skills", credentialId: "qwen-key" });
  expect(props.setProfiles).toHaveBeenCalledWith([expect.objectContaining({ name: "My index" })]);
});

it("does not use an agent-only credential for profile creation", () => {
  render(<EmbeddingProfilesPanel {...props} keys={[{ ...key, purpose: "agent" }]} />);
  expect(mocks.getEmbeddingProfileDefaults).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: "创建 Profile" })).not.toBeInTheDocument();
});

it("shows the profile name, description, provider and model without internal configuration", () => {
  const profile: EmbeddingProfile = {
    profileId: "internal-profile-id", name: "My index", description: "My skills",
    provider: "qwen", model: "text-embedding-v4", modelVersion: "1",
    dimensions: 1024, inputSchemaVersion: "schema-v1", chunkPolicyVersion: "chunk-v1",
    status: "ready", isActive: false, createdAt: 1,
  };
  render(<EmbeddingProfilesPanel {...props} keys={[]} profiles={[profile]} />);
  const card = within(screen.getByRole("heading", { name: "My index" }).closest("article")!);
  for (const text of ["My skills", "服务商", "qwen", "模型", "text-embedding-v4"]) {
    expect(card.getByText(text)).toBeInTheDocument();
  }
  for (const text of ["版本", "维度", "输入结构", "schema-v1", "1024", "internal-profile-id"]) {
    expect(card.queryByText(text)).not.toBeInTheDocument();
  }
  expect(card.getByRole("button", { name: "删除 My index Profile" })).toBeEnabled();
  expect(card.getByRole("button", { name: "激活 Ready Profile" })).toBeEnabled();
});

it("shows configuration errors instead of remaining stuck in loading", async () => {
  mocks.getEmbeddingProfileDefaults.mockRejectedValue(new Error("configuration unavailable"));
  render(<EmbeddingProfilesPanel {...props} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("configuration unavailable");
});
