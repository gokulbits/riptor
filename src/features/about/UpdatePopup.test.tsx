// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let downloadFails = false;

vi.mock("../../lib/ipc", () => ({
  errorText: (e: unknown) => String(e),
  api: {
    version: () => Promise.resolve("0.1.5"),
    updateDownload: () => {
      calls.push("download");
      return downloadFails ? Promise.reject("offline") : Promise.resolve();
    },
    updateInstall: () => {
      calls.push("install");
      return Promise.resolve();
    },
  },
}));

const { useApp } = await import("../../stores/app");
const { UpdatePopup } = await import("./UpdatePopup");

const update = { version: "0.1.6", notes: "Faster and safer.", date: null };

beforeEach(() => {
  calls.length = 0;
  downloadFails = false;
  useApp.setState({ update: null, updateStage: "available", updateProgress: null, updatePopup: null });
});
afterEach(cleanup);

describe("UpdatePopup", () => {
  it("offers the update and downloads it in the background", async () => {
    useApp.getState().setUpdate(update);
    useApp.getState().setUpdatePopup("available");
    render(<UpdatePopup />);
    expect(screen.getByText(/Riptor 0.1.6 is available/)).toBeTruthy();
    expect(screen.getByText("Faster and safer.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Download in the background/ }));
    expect(calls).toEqual(["download"]);
    expect(useApp.getState().updateStage).toBe("downloading");
    expect(useApp.getState().updatePopup).toBeNull();
  });

  it("goes back to available if the download fails", async () => {
    downloadFails = true;
    useApp.getState().setUpdate(update);
    useApp.getState().setUpdatePopup("available");
    render(<UpdatePopup />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Download in the background/ }));
    });
    expect(useApp.getState().updateStage).toBe("available");
  });

  it("asks before installing a downloaded update", async () => {
    useApp.getState().setUpdate(update);
    useApp.getState().setUpdateStage("ready");
    useApp.getState().setUpdatePopup("ready");
    render(<UpdatePopup />);
    expect(screen.getByText(/0.1.6 has been downloaded/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(calls).toEqual([]);
    expect(useApp.getState().updatePopup).toBeNull();

    act(() => useApp.getState().setUpdatePopup("ready"));
    fireEvent.click(screen.getByRole("button", { name: /Restart and install/ }));
    expect(calls).toEqual(["install"]);
  });
});
