import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { Spinner } from "../Spinner";

describe("Spinner", () => {
  it("renders with default size", () => {
    render(() => <Spinner />);
    const spinner = screen.getByLabelText("Loading");
    expect(spinner).toBeDefined();
    expect(spinner.className).toContain("h-6");
  });

  it("renders with sm size", () => {
    render(() => <Spinner size="sm" />);
    expect(screen.getByLabelText("Loading").className).toContain("h-4");
  });

  it("renders with lg size", () => {
    render(() => <Spinner size="lg" />);
    expect(screen.getByLabelText("Loading").className).toContain("h-8");
  });

  it("uses custom label", () => {
    render(() => <Spinner label="Saving" />);
    expect(screen.getByLabelText("Saving")).toBeDefined();
  });

  it("has role status for accessibility", () => {
    render(() => <Spinner />);
    expect(screen.getByRole("status")).toBeDefined();
  });
});
