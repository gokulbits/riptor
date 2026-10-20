import { describe, expect, it } from "vitest";
import { parseLinks } from "./links";

const HASH = "0123456789ABCDEF0123456789abcdef01234567";

describe("parseLinks", () => {
  it("finds magnets, hashes and torrent links", () => {
    const got = parseLinks(`see magnet:?xt=urn:btih:${HASH} and ${HASH}\nhttps://e.org/a.torrent https://e.org/page`);
    expect(got).toEqual([
      { kind: "magnet", value: `magnet:?xt=urn:btih:${HASH}` },
      { kind: "magnet", value: `magnet:?xt=urn:btih:${HASH.toLowerCase()}` },
      { kind: "url", value: "https://e.org/a.torrent" },
    ]);
  });

  it("takes a lone link as meant", () => {
    expect(parseLinks("  https://e.org/download?id=5  ")).toEqual([{ kind: "url", value: "https://e.org/download?id=5" }]);
  });

  it("ignores everything else", () => {
    expect(parseLinks("hello world")).toEqual([]);
    expect(parseLinks("javascript:alert(1) file:///etc/passwd")).toEqual([]);
    expect(parseLinks(`${HASH}0`)).toEqual([]);
  });

  it("dedupes and caps", () => {
    expect(parseLinks(`${HASH} ${HASH}`)).toHaveLength(1);
    const many = Array.from({ length: 80 }, (_, i) => i.toString(16).padStart(40, "0")).join(" ");
    expect(parseLinks(many)).toHaveLength(50);
  });
});
