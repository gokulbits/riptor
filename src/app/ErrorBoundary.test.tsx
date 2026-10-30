// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

function Broken(): never {
  throw new Error("bad torrent row");
}

afterEach(cleanup);

describe("ErrorBoundary", () => {
  it("shows its children when nothing is wrong", () => {
    render(
      <ErrorBoundary>
        <p>Transfers</p>
      </ErrorBoundary>,
    );
    expect(screen.getByText("Transfers")).toBeTruthy();
  });

  it("replaces a crashed window with the error and a way back", () => {
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    render(
      <ErrorBoundary>
        <Broken />
      </ErrorBoundary>,
    );
    quiet.mockRestore();
    expect(screen.getByRole("alert").textContent).toContain("bad torrent row");
    expect(screen.getByRole("button", { name: "Reload window" })).toBeTruthy();
  });
});
