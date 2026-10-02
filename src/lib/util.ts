export function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i >= 0 ? p.slice(i + 1) : p;
}

export function dirname(p: string): string {
  const i = p.lastIndexOf("/");
  return i >= 0 ? p.slice(0, i) : "";
}

export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function dayGroup(ts: number, now = Date.now()): string {
  const d = new Date(ts);
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const diffDays = Math.floor((today.getTime() - new Date(d).setHours(0, 0, 0, 0)) / 86_400_000);
  if (diffDays <= 0) return "Today";
  if (diffDays === 1) return "Yesterday";
  if (diffDays < 7) return "Previous 7 days";
  if (diffDays < 30) return "Previous 30 days";
  return d.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

/**
 * Split markdown into top-level blocks at blank lines that are outside fenced code.
 * While a reply streams, only the last block changes, so earlier blocks can be memoized
 * and never re-parsed.
 */
export function splitMarkdownBlocks(md: string): string[] {
  const lines = md.split("\n");
  const blocks: string[] = [];
  let cur: string[] = [];
  let fence: string | null = null;
  for (const line of lines) {
    const m = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (m) {
      const marker = m[1]!;
      if (fence === null) fence = marker;
      else if (marker[0] === fence[0] && marker.length >= fence.length && line.trim() === marker) fence = null;
    }
    if (fence === null && line.trim() === "" && cur.length > 0) {
      blocks.push(cur.join("\n"));
      cur = [];
      continue;
    }
    if (cur.length === 0 && line.trim() === "" && fence === null) continue;
    cur.push(line);
  }
  if (cur.length) blocks.push(cur.join("\n"));
  return blocks;
}

/** `src/main.rs:42` or `src/main.rs:42:7` or `src/main.rs` (must look like a path). */
export function parsePathRef(s: string): { path: string; line?: number } | null {
  const m = /^([\w@.\-/]+\.[\w]+)(?::(\d+))?(?::\d+)?$/.exec(s.trim());
  if (!m || m[1]!.startsWith("http")) return null;
  if (!m[1]!.includes("/") && !m[2]) return null; // bare "foo.rs" is too ambiguous
  return { path: m[1]!.replace(/^\.\//, ""), line: m[2] ? Number(m[2]) : undefined };
}

const ICONS: Record<string, [string, string]> = {
  rs: ["RS", "#dea584"],
  ts: ["TS", "#3178c6"],
  tsx: ["TX", "#3178c6"],
  js: ["JS", "#e5c33b"],
  jsx: ["JX", "#e5c33b"],
  mjs: ["JS", "#e5c33b"],
  json: ["{}", "#cbcb41"],
  py: ["PY", "#4b8bbe"],
  go: ["GO", "#00add8"],
  md: ["M↓", "#7a8aa6"],
  html: ["<>", "#e44d26"],
  css: ["#", "#563d7c"],
  scss: ["#", "#c6538c"],
  toml: ["TL", "#9c4221"],
  yaml: ["YL", "#cb171e"],
  yml: ["YL", "#cb171e"],
  sh: ["$", "#89e051"],
  c: ["C", "#555fbf"],
  h: ["H", "#555fbf"],
  cpp: ["C+", "#f34b7d"],
  java: ["JV", "#b07219"],
  kt: ["KT", "#a97bff"],
  rb: ["RB", "#cc342d"],
  php: ["PH", "#777bb4"],
  cs: ["C#", "#178600"],
  swift: ["SW", "#f05138"],
  lock: ["LK", "#7a8aa6"],
  svg: ["SV", "#ffb13b"],
  png: ["IM", "#a074c4"],
  jpg: ["IM", "#a074c4"],
  txt: ["TX", "#7a8aa6"],
};

export function fileBadge(name: string): [string, string] {
  const lower = name.toLowerCase();
  if (lower === "dockerfile") return ["DK", "#2496ed"];
  if (lower === "makefile") return ["MK", "#6d8086"];
  if (lower.startsWith(".git")) return ["GI", "#f14e32"];
  const ext = lower.includes(".") ? lower.slice(lower.lastIndexOf(".") + 1) : "";
  return ICONS[ext] ?? ["", "#7a8aa6"];
}
