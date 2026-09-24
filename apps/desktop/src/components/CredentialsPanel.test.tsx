import { createRef, useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { CredentialsPanel } from "./CredentialsPanel";
import type { ApiKeyMetadata } from "../types";
const mocks = vi.hoisted(() => ({ saveApiKey: vi.fn() }));
vi.mock("../api", () => ({ api: mocks }));

it("stores the same secret separately for each selected purpose", async () => {
  mocks.saveApiKey.mockImplementation(async (provider, _secret, purpose) => ({
    id: purpose, provider, purpose, maskedKey: "test-mask", savedAt: 1,
    isAgentActive: false, isEmbeddingActive: false,
  }));
  function Harness() {
    const [keys, setKeys] = useState<ApiKeyMetadata[]>([]);
    return <CredentialsPanel isNative keys={keys} setKeys={setKeys} keyInputRef={createRef()} notify={vi.fn()} requestProfileSwitch={vi.fn()} />;
  }
  render(<Harness />);
  const user = userEvent.setup();
  await user.selectOptions(screen.getByLabelText("储存用途"), "embedding");
  expect(screen.getByLabelText("服务商")).toHaveValue("openai");
  expect(within(screen.getByLabelText("服务商")).queryByText("DeepSeek")).not.toBeInTheDocument();
  await user.type(screen.getByLabelText("API Key"), "test-secret-only");
  await user.click(screen.getByRole("button", { name: "安全保存" }));
  expect(mocks.saveApiKey).toHaveBeenLastCalledWith("openai", "test-secret-only", "embedding");
  const agent = screen.getByText("Agent 凭据").closest("section")!;
  const embedding = screen.getByText("Embedding 凭据").closest("section")!;
  expect(within(agent).queryByText("test-mask")).not.toBeInTheDocument();
  expect(within(embedding).getByText("test-mask")).toBeInTheDocument();
  expect(within(embedding).getByRole("button", { name: "删除" })).toBeEnabled();
  await user.selectOptions(screen.getByLabelText("储存用途"), "agent");
  await user.type(screen.getByLabelText("API Key"), "test-secret-only");
  await user.click(screen.getByRole("button", { name: "安全保存" }));
  expect(mocks.saveApiKey).toHaveBeenLastCalledWith("openai", "test-secret-only", "agent");
  expect(within(agent).getAllByText("test-mask")).toHaveLength(1);
  expect(within(embedding).getAllByText("test-mask")).toHaveLength(1);
});
