import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import { Button } from "../Button";

describe("Button", () => {
  it("renders with default primary variant", () => {
    render(() => <Button>Click me</Button>);
    const btn = screen.getByRole("button", { name: "Click me" });
    expect(btn).toBeDefined();
    expect(btn.className).toContain("bg-primary-600");
  });

  it("renders destructive variant", () => {
    render(() => <Button variant="destructive">Delete</Button>);
    expect(screen.getByRole("button").className).toContain("bg-error-600");
  });

  it("renders ghost variant", () => {
    render(() => <Button variant="ghost">Ghost</Button>);
    expect(screen.getByRole("button").className).toContain("bg-transparent");
  });

  it("renders outline variant", () => {
    render(() => <Button variant="outline">Outline</Button>);
    expect(screen.getByRole("button").className).toContain("border");
  });

  it("renders secondary variant", () => {
    render(() => <Button variant="secondary">Secondary</Button>);
    expect(screen.getByRole("button").className).toContain("bg-secondary-100");
  });

  it("shows spinner and disables when loading", () => {
    render(() => <Button loading>Loading</Button>);
    const btn = screen.getByRole("button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(screen.getByLabelText("Loading")).toBeDefined();
  });

  it("is disabled when disabled prop is true", () => {
    render(() => <Button disabled>Disabled</Button>);
    expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(
      true
    );
  });

  it("calls onClick when clicked", () => {
    const onClick = vi.fn();
    render(() => <Button onClick={onClick}>Click</Button>);
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("does not call onClick when disabled", () => {
    const onClick = vi.fn();
    render(() => (
      <Button onClick={onClick} disabled>
        Click
      </Button>
    ));
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).not.toHaveBeenCalled();
  });

  it("merges custom class names", () => {
    render(() => <Button class="custom-class">Styled</Button>);
    expect(screen.getByRole("button").className).toContain("custom-class");
  });
});
