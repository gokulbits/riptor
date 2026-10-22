// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ImportScan } from "../../lib/types";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

let scanResult: ImportScan;
const imported: string[] = [];

vi.mock("../../lib/ipc", () => ({
  errorText: (e: unknown) => String(e),
  api: {
    importScan: () => Promise.resolve(scanResult),
    importTorrents: (path: string) => {
      imported.push(path);
      return Promise.resolve({ added: 3, already_present: 1, skipped: 0, failed: [], all_time: { downloaded: 2048, uploaded: 4096 } });
    },
  },
}));

const { ImportDialog } = await import("./ImportDialog");

beforeEach(() => {
  imported.length = 0;
});
afterEach(cleanup);

describe("ImportDialog", () => {
  it("finds qBittorrent's torrents and imports them", async () => {
    scanResult = { dir: "C:\\Users\\me\\AppData\\Local\\qBittorrent\\BT_backup", found: 4, skipped: 0, all_time: { downloaded: 2048, uploaded: 4096 } };
    render(<ImportDialog onClose={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Import 4 torrents" }));
    expect(await screen.findByText(/3 torrents imported/)).toBeTruthy();
    expect(screen.getByText(/1 was already in Riptor/)).toBeTruthy();
    expect(screen.getByText(/all-time totals \(2\.0 KiB downloaded, 4\.0 KiB uploaded\)/)).toBeTruthy();
    expect(imported).toEqual([scanResult.dir]);
  });

  it("explains where to look when nothing is found", async () => {
    scanResult = { dir: null, found: 0, skipped: 0, all_time: null };
    render(<ImportDialog onClose={() => {}} />);
    expect(await screen.findByText(/couldn't find qBittorrent's torrents/)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Import" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("button", { name: /Choose folder/ })).toBeTruthy();
  });
});
