import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "../Card";

describe("Card", () => {
  it("renders with content", () => {
    render(() => (
      <Card>
        <CardHeader>
          <CardTitle>Title</CardTitle>
          <CardDescription>Description</CardDescription>
        </CardHeader>
        <CardContent>Body content</CardContent>
        <CardFooter>Footer content</CardFooter>
      </Card>
    ));

    expect(screen.getByText("Title")).toBeDefined();
    expect(screen.getByText("Description")).toBeDefined();
    expect(screen.getByText("Body content")).toBeDefined();
    expect(screen.getByText("Footer content")).toBeDefined();
  });

  it("applies hoverable variant class", () => {
    const { container } = render(() => (
      <Card variant="hoverable">Hover me</Card>
    ));
    const card = container.querySelector(".hover\\:shadow-dropdown");
    expect(card).not.toBeNull();
  });

  it("merges custom class names", () => {
    const { container } = render(() => <Card class="my-card">Content</Card>);
    const card = container.querySelector(".my-card");
    expect(card).not.toBeNull();
  });
});
