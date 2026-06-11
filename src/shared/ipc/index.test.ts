import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { commands } from "./index";

const invokeMock = vi.mocked(invoke);

describe("ipc commands", () => {
  it("createNote forwards the input under the `input` key", async () => {
    invokeMock.mockResolvedValue({
      id: "x",
      title: "T",
      body: "B",
      created_at: 0,
      updated_at: 0,
    });
    await commands.createNote({ title: "T", body: "B" });
    expect(invokeMock).toHaveBeenCalledWith("create_note", {
      input: { title: "T", body: "B" },
    });
  });

  it("getNote forwards the id", async () => {
    invokeMock.mockResolvedValue(null);
    await commands.getNote("abc");
    expect(invokeMock).toHaveBeenCalledWith("get_note", { id: "abc" });
  });
});
