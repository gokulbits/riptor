import type { Section } from "./SettingsDialog";

export interface SettingEntry {
  section: Section;
  /** The heading it sits under, as written in the page. */
  group: string;
  /** The label as written in the page; a group's own name when it points at the whole group. */
  label: string;
  /** Words people may type that the label doesn't contain. */
  keywords: string;
}

const SECTION_NAMES: Record<Section, string> = {
  downloads: "Downloads",
  connection: "Connection",
  speed: "Speed",
  bittorrent: "Seeding & Queue",
  storage: "Disk Cache",
  privacy: "Privacy & Security",
  apps: "Default App",
  backup: "Backup & Restore",
  remote: "Remote Control",
  about: "About",
};

export const sectionName = (s: Section) => SECTION_NAMES[s];

const e = (section: Section, group: string, label: string, keywords = ""): SettingEntry => ({ section, group, label, keywords });

export const ENTRIES: SettingEntry[] = [
  e("downloads", "Saving", "Default download folder", "save location directory path where files go"),
  e("downloads", "Saving", "Keep incomplete downloads in", "temporary partial unfinished temp folder move when complete"),
  e("downloads", "Saving", "Put multi-file torrents in their own folder", "subfolder directory create name"),
  e("downloads", "Saving", "Add new torrents stopped", "paused start automatically manual"),
  e("downloads", "Saving", "Pause downloads below this much free space (MiB)", "disk full low space minimum free gigabytes drive"),
  e("downloads", "Saving", "Move files when a torrent's category changes", "relocate category folder"),
  e("downloads", "Saving", "Mark downloaded files as coming from the internet", "mark of the web motw smartscreen windows security zone"),
  e("downloads", "Watched folders", "Watched folders", "auto add torrent files folder monitor directory .torrent"),
  e("downloads", "Window", "Keep running in the tray when the window is closed", "system tray minimize close background exit quit"),
  e("downloads", "Window", "Start Riptor when I sign in", "autostart startup boot login launch windows"),

  e("connection", "Listening", "Port", "listen incoming tcp firewall random 6881 router"),
  e("connection", "Listening", "Protocol", "tcp utp transport"),
  e("connection", "Listening", "Listen on address", "bind interface adapter ip network vpn"),
  e("connection", "Listening", "Forward the port with UPnP", "nat port forwarding router igd"),
  e("connection", "Peers", "Encrypt peer traffic", "encryption obfuscation mse pe protocol require prefer"),
  e("connection", "Peers", "DHT (find peers without trackers)", "distributed hash table trackerless"),
  e("connection", "Peers", "Local peer discovery", "lsd lan local network multicast"),
  e("connection", "Peers", "Web seeds", "http sources seeds"),
  e("connection", "Peers", "Show peer countries", "geoip flags location country db-ip"),
  e("connection", "Peers", "Max peers per torrent", "connections limit peer limit"),
  e("connection", "Peers", "SOCKS5 proxy for peer connections", "proxy socks tor"),
  e("connection", "Trackers", "Add these trackers to new torrents", "extra trackers announce list public"),

  e("speed", "Global limits (KiB/s)", "Download", "download speed limit bandwidth throttle cap rate"),
  e("speed", "Global limits (KiB/s)", "Upload", "upload speed limit bandwidth throttle cap rate"),
  e("speed", "Alternative limits (KiB/s)", "Download", "alternative download speed limit turtle slow"),
  e("speed", "Alternative limits (KiB/s)", "Upload", "alternative upload speed limit turtle slow"),
  e("speed", "Alternative limits (KiB/s)", "Use alternative limits now", "turtle mode slow speed switch"),
  e("speed", "Schedule", "Use alternative limits on a schedule", "scheduler timer time day night"),
  e("speed", "Schedule", "From", "start time hour schedule"),
  e("speed", "Schedule", "To", "end time hour schedule"),
  e("speed", "Data use", "Pause torrents on metered connections", "mobile data hotspot cellular metered network"),
  e("speed", "Data use", "Limit how much Riptor transfers each month", "data cap quota monthly traffic gigabytes"),
  e("speed", "Data use", "Limit (GiB)", "cap monthly quota"),
  e("speed", "Data use", "Counts", "download upload both direction"),
  e("speed", "Data use", "Resets on day", "billing cycle reset day month"),

  e("bittorrent", "Seeding limits", "Max ratio", "share ratio upload limit seed goal"),
  e("bittorrent", "Seeding limits", "Max seeding time (minutes)", "seed time duration"),
  e("bittorrent", "Seeding limits", "Max inactive time (minutes)", "idle inactive seeding"),
  e("bittorrent", "Seeding limits", "When a limit is reached", "action stop remove delete pause"),
  e("bittorrent", "Queueing", "Limit how many torrents run at once", "queue active slots"),
  e("bittorrent", "Queueing", "Active downloads", "queue slots maximum"),
  e("bittorrent", "Queueing", "Active uploads", "queue slots maximum seeding"),
  e("bittorrent", "Queueing", "Active torrents", "queue slots maximum total"),
  e("bittorrent", "Queueing", "Don't count slow torrents in these limits", "slow inactive ignore"),
  e("bittorrent", "Queueing", "Download threshold (KiB/s)", "slow rate queue"),
  e("bittorrent", "Queueing", "Upload threshold (KiB/s)", "slow rate queue"),
  e("bittorrent", "Queueing", "Inactivity timer (seconds)", "idle slow queue"),
  e("bittorrent", "Looking after what you seed", "Check finished data in the background", "scrub verify integrity recheck corruption disk health"),
  e("bittorrent", "Looking after what you seed", "Every (days)", "scrub interval how often"),
  e("bittorrent", "Looking after what you seed", "Read at most (MiB/s)", "scrub speed disk throttle"),
  e("bittorrent", "Looking after what you seed", "Seed where it matters", "rare scarce seeders priority"),

  e("storage", "Disk cache", "Read cache (MiB)", "memory ram upload cache"),
  e("storage", "Disk cache", "Write buffer (MiB)", "memory ram write piece buffer"),
  e("storage", "Disk cache", "Max open files", "file handles descriptors limit ulimit"),
  e("storage", "Disk cache", "Disk space allocation", "preallocate sparse full fragmentation"),
  e("storage", "Disk cache", "Torrents to check at once", "hash check recheck parallel verification hdd ssd"),

  e("privacy", "Privacy", "Anonymous mode", "hide client name disable dht upnp lsd"),
  e("privacy", "Untrusted networks", "Require the VPN on networks I haven't trusted", "kill switch wifi trusted network public vpn required"),
  e("privacy", "IP blocklist", "Blocklist file", "ipfilter p2p dat block ip ranges"),
  e("privacy", "IP blocklist", "Blocklist URL", "ipfilter download update block list"),
  e("privacy", "Banned peers", "Banned peers", "ban unban ip peers block"),

  e("apps", "Default app", "Default app", "default program file association open with torrent magnet handler"),
  e("apps", "Default app", "Opens .torrent files", "file association default handler"),
  e("apps", "Default app", "Opens magnet links from your browser", "magnet protocol handler association url"),

  e("backup", "Create a backup", "Create a backup", "export save copy archive"),
  e("backup", "Create a backup", "Protect with a password", "encrypt backup secure"),
  e("backup", "Restore", "Restore", "import load backup file recover"),
  e("backup", "Restore", "Resume torrents that were running", "restore start after"),
  e("backup", "Restore", "Torrents", "restore choose what"),
  e("backup", "Restore", "Settings", "restore choose what preferences"),
  e("backup", "Restore", "Statistics", "restore choose what history"),

  e("remote", "Web user interface (remote control)", "Allow remote control", "web ui api qbitcontroller phone app enable server"),
  e("remote", "Web user interface (remote control)", "IP address", "bind listen interface"),
  e("remote", "Web user interface (remote control)", "Port", "listen web port"),
  e("remote", "Web user interface (remote control)", "Accept connections from outside the local network", "internet lan external public"),
  e("remote", "Web user interface (remote control)", "Use UPnP to forward the port from my router", "nat port forwarding"),
  e("remote", "HTTPS", "Use HTTPS instead of HTTP", "tls ssl encrypt certificate"),
  e("remote", "HTTPS", "Certificate (PEM)", "tls ssl cert file"),
  e("remote", "HTTPS", "Key (PEM)", "tls ssl private key file"),
  e("remote", "Authentication", "Username", "login user name remote control"),
  e("remote", "Authentication", "Password", "login remote control sign in"),
  e("remote", "Authentication", "API key", "token access generate copy remove"),
  e("remote", "Authentication", "Bypass authentication for clients on localhost", "local skip login"),
  e("remote", "Authentication", "Bypass authentication for clients in whitelisted IP subnets", "skip login trusted network cidr"),
  e("remote", "Authentication", "IP subnet whitelist", "allow list cidr trusted"),
  e("remote", "Authentication", "Ban client after consecutive failures", "brute force lockout failed attempts"),
  e("remote", "Authentication", "Ban for (minutes)", "lockout duration"),
  e("remote", "Authentication", "Session timeout (minutes)", "log out expire idle"),
  e("remote", "Security", "Enable clickjacking protection", "frame x-frame-options header"),
  e("remote", "Security", "Enable Cross-Site Request Forgery (CSRF) protection", "csrf origin referer"),
  e("remote", "Security", "Enable cookie Secure flag (requires HTTPS)", "session cookie"),
  e("remote", "Security", "Enable Host header validation", "dns rebinding hostname"),
  e("remote", "Security", "Server domains", "host header allowed hostnames"),
  e("remote", "Custom HTTP headers", "Add custom HTTP headers", "response headers cors"),
  e("remote", "Reverse proxy", "Enable reverse proxy support", "nginx forwarded for proxy"),
  e("remote", "Reverse proxy", "Trusted proxies", "x-forwarded-for allowed"),
  e("remote", "Dynamic DNS", "Update my dynamic domain name", "ddns dyndns no-ip hostname"),
  e("remote", "Dynamic DNS", "Provider's update address", "ddns service url"),
  e("remote", "Dynamic DNS", "Domain name", "ddns hostname"),

  e("about", "About", "Check automatically when Riptor starts", "updates update check version upgrade"),
  e("about", "About", "Report a problem", "bug issue github feedback support"),
  e("about", "About", "Source code", "github repository open source"),
  e("about", "About", "Security policy", "vulnerability report disclosure"),
  e("about", "About", "Contributors", "credits authors thanks"),
  e("about", "About", "Software shipped with Riptor", "licenses third party notices credits open source"),
];

// ---------------------------------------------------------------- the index

const WEIGHT = { label: 10, keywords: 6, group: 3, section: 2 } as const;
const PREFIX = 0.6;

/** Lowercase words with accents and a trailing plural s dropped, so "limits" finds "limit". */
export function tokens(text: string): string[] {
  return text
    .replace(/[µμ]/g, "u")
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean)
    .map((w) => (w.length > 3 && w.endsWith("s") && !w.endsWith("ss") ? w.slice(0, -1) : w));
}

type Posting = { doc: number; weight: number };

let built: { postings: Map<string, Posting[]>; vocabulary: string[] } | null = null;

function build() {
  const postings = new Map<string, Posting[]>();
  ENTRIES.forEach((entry, doc) => {
    const best = new Map<string, number>();
    const add = (text: string, weight: number) => {
      for (const t of tokens(text)) best.set(t, Math.max(best.get(t) ?? 0, weight));
    };
    add(entry.label, WEIGHT.label);
    add(entry.keywords, WEIGHT.keywords);
    add(entry.group, WEIGHT.group);
    add(SECTION_NAMES[entry.section], WEIGHT.section);
    for (const [t, weight] of best) {
      const list = postings.get(t);
      if (list) list.push({ doc, weight });
      else postings.set(t, [{ doc, weight }]);
    }
  });
  return { postings, vocabulary: [...postings.keys()] };
}

/** Entries that have every word of the query (a word may be just the start of one), best first. */
export function searchSettings(query: string, limit = 30): SettingEntry[] {
  const words = tokens(query);
  if (words.length === 0) return [];
  built ??= build();
  const { postings, vocabulary } = built;
  let scores: Map<number, number> | null = null;
  for (const [i, word] of words.entries()) {
    const last = i === words.length - 1;
    const found = new Map<number, number>();
    for (const term of vocabulary) {
      // Only the word still being typed may be a prefix.
      const factor = term === word ? 1 : last && term.startsWith(word) ? PREFIX : 0;
      if (!factor) continue;
      for (const { doc, weight } of postings.get(term) ?? []) {
        found.set(doc, Math.max(found.get(doc) ?? 0, weight * factor));
      }
    }
    const next = new Map<number, number>();
    for (const [doc, w] of found) {
      if (scores === null) next.set(doc, w);
      else if (scores.has(doc)) next.set(doc, scores.get(doc)! + w);
    }
    scores = next;
    if (scores.size === 0) return [];
  }
  return [...scores!.entries()]
    .sort((a, b) => b[1] - a[1] || a[0] - b[0])
    .slice(0, limit)
    .map(([doc]) => ENTRIES[doc]);
}
