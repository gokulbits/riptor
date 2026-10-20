// Fetches the OpenVPN MSI the Windows setup bundles and checks its pinned hash.
// When upgrading, check the new MSI is signed by "OpenVPN Inc." before pinning.
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "2.7.8-I001";
const SOURCE = `https://swupdate.openvpn.net/community/releases/OpenVPN-${VERSION}-amd64.msi`;
const SHA256 = "1ea61a3056905b90607d4b0f084c04e5cf3043e07cf192d9cd44b0474f29cc49";
const MAX_BYTES = 32 * 1024 * 1024;

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(root, "src-tauri", "windows", "openvpn", "openvpn.msi");

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

try {
  if (sha256(await readFile(out)) === SHA256) {
    console.log(`OpenVPN ${VERSION} already present`);
    process.exit(0);
  }
} catch {
}

const res = await fetch(SOURCE, { redirect: "follow" });
if (!res.ok) throw new Error(`download failed: HTTP ${res.status}`);
if (new URL(res.url).protocol !== "https:") throw new Error("redirected away from https");
const buf = Buffer.from(await res.arrayBuffer());
if (buf.length > MAX_BYTES) throw new Error("download is too large");
const got = sha256(buf);
if (got !== SHA256) throw new Error(`SHA-256 mismatch: expected ${SHA256}, got ${got}`);

await mkdir(dirname(out), { recursive: true });
await writeFile(`${out}.tmp`, buf);
await rename(`${out}.tmp`, out);
console.log(`OpenVPN ${VERSION} verified and saved to ${out}`);
