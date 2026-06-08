import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("~/shared/ipc", () => ({
  commands: { answerQuestion: vi.fn(), grantCloudConsent: vi.fn(), revokeCloudConsent: vi.fn() },
}));

import { commands } from "~/shared/ipc";
import { qaApi } from "./api";

const mocked = vi.mocked(commands);

describe("qaApi", () => {
  beforeEach(() => vi.clearAllMocks());

  it("ask delegates to answerQuestion with the query", async () => {
    mocked.answerQuestion.mockResolvedValue({ kind: "answer", state: "grounded", text: "x", cited: [] });
    await qaApi.ask("why is the sky blue?");
    expect(mocked.answerQuestion).toHaveBeenCalledWith("why is the sky blue?");
  });

  it("grant delegates to grantCloudConsent with the provider", async () => {
    mocked.grantCloudConsent.mockResolvedValue(null);
    await qaApi.grant("kimi");
    expect(mocked.grantCloudConsent).toHaveBeenCalledWith("kimi");
  });
});
