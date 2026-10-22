// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Settings } from "../../lib/types";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

const saved: Settings[] = [];
let current: Settings;

vi.mock("../../lib/ipc", () => ({
  errorText: (e: unknown) => String(e),
  api: {
    settings: () => Promise.resolve(structuredClone(current)),
    updateSettings: (s: Settings) => {
      saved.push(s);
      return Promise.resolve();
    },
    networkInfo: () => Promise.resolve({ metered: false, names: [] }),
    countryStatus: () => Promise.resolve({ enabled: false, ready: false, updated: null, error: null, attribution: "IP geolocation by DB-IP" }),
    categories: () => Promise.resolve({}),
    autostartGet: () => Promise.resolve(false),
  },
}));

const { SettingsDialog } = await import("./SettingsDialog");

// Only what the Downloads and Connection sections read; the rest must survive untouched.
const fixture = {
  download_dir: "C:\\Downloads",
  incomplete_dir: null,
  create_subfolder: true,
  add_stopped: false,
  min_free_mib: 1024,
  move_on_category_change: false,
  mark_of_the_web: true,
  watched_folders: [],
  watched_folder: null,
  close_to_tray: true,
  listen_port: 6881,
  transport: "tcp",
  listen_address: null,
  upnp: true,
  dht: true,
  lsd: true,
  web_seeds: true,
  peer_countries: false,
  peer_limit: null,
  proxy_url: null,
  extra_trackers: ["udp://tracker.example.org:1337/announce"],
  columns: [{ id: "name", width: 300, visible: true }],
  something_the_ui_never_shows: { nested: [1, 2, 3] },
} as unknown as Settings;

beforeEach(() => {
  current = structuredClone(fixture);
  saved.length = 0;
});
afterEach(cleanup);

const open = async (section?: "connection") => {
  render(<SettingsDialog onClose={() => {}} initialSection={section} />);
  await screen.findByRole("button", { name: "Save" });
};

describe("SettingsDialog", () => {
  it("saves exactly what it loaded when nothing changed", async () => {
    await open();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual(fixture);
  });

  it("changes only the toggled setting", async () => {
    await open("connection");
    fireEvent.click(screen.getByRole("switch", { name: /Show peer countries/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({ ...fixture, peer_countries: true });
  });

  it("keeps edits when switching sections", async () => {
    await open("connection");
    fireEvent.click(screen.getByRole("switch", { name: /DHT/ }));
    fireEvent.click(screen.getByRole("button", { name: "Downloads" }));
    fireEvent.click(screen.getByRole("switch", { name: /Add new torrents stopped/ }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({ ...fixture, dht: false, add_stopped: true });
  });

  it("opens at the section it's asked for", async () => {
    await open("connection");
    expect(screen.getByRole("switch", { name: /Show peer countries/ })).toBeTruthy();
  });
});
