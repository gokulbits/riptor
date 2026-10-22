import type { PeerInfo } from "../../lib/types";

/** Busiest peers first. */
export const sortPeers = (peers: PeerInfo[]) =>
  [...peers].sort((a, b) => b.downloaded + b.uploaded - (a.downloaded + a.uploaded) || a.addr.localeCompare(b.addr));

/** "1.2.3.4:6881" or "[::1]:6881" to the bare IP, for bans. */
export const peerIp = (addr: string) => addr.replace(/^\[?([^\]]+?)\]?:\d+$/, "$1");

// Intl can be missing region data on some systems.
const regionNames = (() => {
  try {
    return new Intl.DisplayNames(undefined, { type: "region" });
  } catch {
    return null;
  }
})();

export const countryName = (code: string) => {
  try {
    return regionNames?.of(code) ?? code;
  } catch {
    return code;
  }
};
