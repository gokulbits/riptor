// Fetches the embeddable Python the Windows setup ships and unpacks it into
// src-tauri/python. The pin lives in crates/riptor-search/src/pyembed.rs.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pin = await readFile(join(root, "crates", "riptor-search", "src", "pyembed.rs"), "utf8");
const block = pin.match(/target_arch = "x86_64"\)\)\]\s*pub const RELEASE[^}]*\}/);
if (!block) throw new Error("x86_64 release pin not found in pyembed.rs");
const url = block[0].match(/url:\s*"([^"]+)"/)?.[1];
const sha = block[0].match(/sha256:\s*"([0-9a-f]{64})"/)?.[1];
if (!url?.startsWith("https://www.python.org/ftp/python/") || !sha) throw new Error("unexpected pin in pyembed.rs");

const outDir = join(root, "src-tauri", "python");
const stamp = join(outDir, ".riptor-sha256");
if (existsSync(join(outDir, "python.exe")) && existsSync(stamp) && (await readFile(stamp, "utf8")).trim() === sha) {
  console.log("embeddable Python already present");
  process.exit(0);
}

const res = await fetch(url, { redirect: "follow" });
if (!res.ok) throw new Error(`download failed: HTTP ${res.status}`);
if (new URL(res.url).protocol !== "https:") throw new Error("redirected away from https");
const buf = Buffer.from(await res.arrayBuffer());
if (buf.length > 32 * 1024 * 1024) throw new Error("download is too large");
const got = createHash("sha256").update(buf).digest("hex");
if (got !== sha) throw new Error(`SHA-256 mismatch: expected ${sha}, got ${got}`);

await rm(outDir, { recursive: true, force: true });
await mkdir(outDir, { recursive: true });
const zip = join(root, "src-tauri", "python-embed.zip");
await writeFile(zip, buf);
try {
  // Windows' bsdtar reads zips; Git Bash's GNU tar doesn't.
  const tar = process.platform === "win32" ? join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe") : "bsdtar";
  execFileSync(tar, ["-xf", zip, "-C", outDir], { stdio: "inherit" });
} finally {
  await rm(zip, { force: true });
}
if (!existsSync(join(outDir, "python.exe"))) throw new Error("python.exe missing after unpacking");
await writeFile(stamp, sha);
console.log(`embeddable Python verified and unpacked to ${outDir}`);
