import { describe, expect, it } from "vitest";
import type { PeerInfo } from "../../lib/types";
import { countryName, peerIp, sortPeers } from "./peers";

const peer = (addr: string, downloaded: number, uploaded = 0): PeerInfo => ({
  addr,
  client: null,
  state: "live",
  conn: "tcp",
  downloaded,
  uploaded,
  country: null,
});

describe("peers", () => {
  it("sorts the busiest peers first, then by address", () => {
    const sorted = sortPeers([peer("b:1", 10), peer("a:1", 10), peer("c:1", 5, 50), peer("d:1", 0)]);
    expect(sorted.map((p) => p.addr)).toEqual(["c:1", "a:1", "b:1", "d:1"]);
  });

  it("doesn't reorder the list it was given", () => {
    const list = [peer("a:1", 0), peer("b:1", 9)];
    sortPeers(list);
    expect(list[0].addr).toBe("a:1");
  });

  it("takes the IP out of v4 and v6 addresses", () => {
    expect(peerIp("203.0.113.5:6881")).toBe("203.0.113.5");
    expect(peerIp("[2001:db8::1]:51413")).toBe("2001:db8::1");
    expect(peerIp("203.0.113.5")).toBe("203.0.113.5");
  });

  it("names countries and falls back to the code", () => {
    expect(countryName("DE")).not.toBe("");
    expect(countryName("not a region")).toBe("not a region");
  });
});
