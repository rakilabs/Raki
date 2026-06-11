import { fireEvent, render, screen } from "@solidjs/testing-library";
import { Search } from "lucide-solid";
import { describe, expect, it } from "vitest";
import { Input } from "../Input";

describe("Input", () => {
  it("renders with label", () => {
    render(() => <Input label="Email" />);
    expect(screen.getByLabelText("Email")).toBeDefined();
  });

  it("shows error message and aria-invalid", () => {
    render(() => <Input label="Name" error="Required" />);
    const input = screen.getByLabelText("Name") as HTMLInputElement;
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByText("Required")).toBeDefined();
  });

  it("shows helper text when no error", () => {
    render(() => <Input label="Username" helper="3-20 characters" />);
    expect(screen.getByText("3-20 characters")).toBeDefined();
  });

  it("does not show helper when error is present", () => {
    render(() => (
      <Input label="Username" helper="3-20 characters" error="Too short" />
    ));
    expect(screen.queryByText("3-20 characters")).toBeNull();
    expect(screen.getByText("Too short")).toBeDefined();
  });

  it("updates value on input", () => {
    render(() => <Input label="Search" />);
    const input = screen.getByLabelText("Search") as HTMLInputElement;
    fireEvent.input(input, { target: { value: "hello" } });
    expect(input.value).toBe("hello");
  });

  it("is disabled when disabled prop is true", () => {
    render(() => <Input label="Locked" disabled />);
    expect(screen.getByLabelText("Locked").hasAttribute("disabled")).toBe(true);
  });

  it("renders left icon slot", () => {
    render(() => (
      <Input label="Search" leftIcon={<Search data-testid="search-icon" />} />
    ));
    expect(screen.getByTestId("search-icon")).toBeDefined();
  });
});
