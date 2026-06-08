import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, screen, waitFor } from "@solidjs/testing-library";

vi.mock("./api", () => ({ qaApi: { ask: vi.fn(), grant: vi.fn(), revoke: vi.fn() } }));

import { qaApi } from "./api";
import { AskBox } from "./AskBox";

const mocked = vi.mocked(qaApi);

describe("AskBox", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocked.ask.mockReset();
    mocked.grant.mockReset();
    mocked.revoke.mockReset();
  });

  it("asking renders a consent preview without sending", async () => {
    mocked.ask.mockResolvedValue({
      kind: "needs_consent",
      preview: { provider: "kimi", summary: "1 sources, 10 tokens → kimi/k2", source_titles: ["Trip"] },
    });
    render(() => <AskBox />);
    fireEvent.input(screen.getByPlaceholderText(/Ask a question/i), { target: { value: "how do I pay?" } });
    fireEvent.submit(screen.getByRole("button", { name: "Ask" }).closest("form")!);
    await waitFor(() => screen.getByText(/This will send to the cloud/i));
    expect(screen.getByText("Trip")).toBeDefined();
    expect(mocked.grant).not.toHaveBeenCalled();
  });

  it("confirming sends: grants then re-asks, then shows the answer", async () => {
    mocked.ask
      .mockResolvedValueOnce({
        kind: "needs_consent",
        preview: { provider: "kimi", summary: "s", source_titles: ["Trip"] },
      })
      .mockResolvedValueOnce({ kind: "answer", state: "grounded", text: "Pay cash.", cited: [{ id: "n1", title: "Trip" }] });
    mocked.grant.mockResolvedValue(null);
    render(() => <AskBox />);
    fireEvent.input(screen.getByPlaceholderText(/Ask a question/i), { target: { value: "pay?" } });
    fireEvent.submit(screen.getByRole("button", { name: "Ask" }).closest("form")!);
    await waitFor(() => screen.getByRole("button", { name: "Send to cloud" }));
    fireEvent.click(screen.getByRole("button", { name: "Send to cloud" }));
    await waitFor(() => expect(mocked.grant).toHaveBeenCalledWith("kimi"));
    expect(mocked.ask).toHaveBeenCalledTimes(2);
    await waitFor(() => screen.getByText("Pay cash."));
  });

  it("renders an error alert when ask rejects", async () => {
    mocked.ask.mockRejectedValue({ kind: "provider", message: "boom" });
    render(() => <AskBox />);
    fireEvent.input(screen.getByPlaceholderText(/Ask a question/i), { target: { value: "x" } });
    fireEvent.submit(screen.getByRole("button", { name: "Ask" }).closest("form")!);
    await waitFor(() => screen.getByRole("alert"));
    expect(screen.getByRole("alert").textContent).toContain("boom");
  });
});
