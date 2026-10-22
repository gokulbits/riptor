// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Dialog } from "./ui";

afterEach(cleanup);

describe("Dialog", () => {
  it("closes only the topmost dialog on Escape", () => {
    const outer = vi.fn();
    const inner = vi.fn();
    render(
      <Dialog title="Settings" onClose={outer}>
        <Dialog title="Logs" onClose={inner}>
          logs
        </Dialog>
      </Dialog>,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(inner).toHaveBeenCalledTimes(1);
    expect(outer).not.toHaveBeenCalled();
  });

  it("closes a lone dialog on Escape", () => {
    const close = vi.fn();
    render(
      <Dialog title="Settings" onClose={close}>
        body
      </Dialog>,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(close).toHaveBeenCalledTimes(1);
  });
});
