import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { Badge } from "../Badge";

describe("Badge", () => {
  it("renders with default variant", () => {
    render(() => <Badge>New</Badge>);
    expect(screen.getByText("New")).toBeDefined();
  });

  it("renders destructive variant", () => {
    render(() => <Badge variant="destructive">Error</Badge>);
    const badge = screen.getByText("Error");
    expect(badge.className).toContain("bg-error-100");
  });

  it("renders success variant", () => {
    render(() => <Badge variant="success">Done</Badge>);
    const badge = screen.getByText("Done");
    expect(badge.className).toContain("bg-success-100");
  });

  it("renders outline variant", () => {
    render(() => <Badge variant="outline">Outline</Badge>);
    const badge = screen.getByText("Outline");
    expect(badge.className).toContain("border-border");
  });

  it("merges custom class names", () => {
    render(() => <Badge class="extra-class">Styled</Badge>);
    expect(screen.getByText("Styled").className).toContain("extra-class");
  });
});
