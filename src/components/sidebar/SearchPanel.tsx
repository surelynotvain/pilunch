import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../Icon";
import { FileBadge } from "./Explorer";
import { useApp } from "../../store/app";
import { useEditor } from "../../store/editor";
import { api, errorText } from "../../lib/ipc";
import { basename, dirname } from "../../lib/util";
import type { GrepMatch, GrepResult } from "../../lib/types";

function HitText({ m }: { m: GrepMatch }) {
  // Trim leading whitespace but keep ranges aligned.
  const lead = m.text.length - m.text.trimStart().length;
  const text = m.text.slice(lead);
  const parts: React.ReactNode[] = [];
  let pos = 0;
  for (const [s0, e0] of m.ranges) {
    const s = Math.max(0, s0 - lead);
    const e = Math.max(0, e0 - lead);
    if (s > pos) parts.push(text.slice(pos, s));
    parts.push(<mark key={s}>{text.slice(s, e)}</mark>);
    pos = e;
  }
  parts.push(text.slice(pos));
  return <>{parts}</>;
}

export function SearchPanel() {
  const workspace = useApp((s) => s.workspace);
  const [query, setQuery] = useState("");
  const [include, setInclude] = useState("");
  const [opts, setOpts] = useState({ regex: false, caseSensitive: false, wholeWord: false });
  const [result, setResult] = useState<GrepResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const input = useRef<HTMLInputElement>(null);
  const seq = useRef(0);

  useEffect(() => {
    input.current?.focus();
    const onFocus = () => {
      input.current?.focus();
      input.current?.select();
    };
    window.addEventListener("pilunch-focus-search", onFocus);
    return () => window.removeEventListener("pilunch-focus-search", onFocus);
  }, []);

  useEffect(() => {
    if (!workspace || !query) {
      setResult(null);
      setError(null);
      return;
    }
    const my = ++seq.current;
    const t = window.setTimeout(async () => {
      setBusy(true);
      try {
        const r = await api.searchText(query, {
          regex: opts.regex,
          caseInsensitive: !opts.caseSensitive,
          wholeWord: opts.wholeWord,
          include: include.trim() || null,
          maxResults: 3000,
        });
        if (my === seq.current) {
          setResult(r);
          setError(null);
        }
      } catch (e) {
        if (my === seq.current) setError(errorText(e));
      } finally {
        if (my === seq.current) setBusy(false);
      }
    }, 220);
    return () => window.clearTimeout(t);
  }, [query, include, opts, workspace]);

  const groups = useMemo(() => {
    const m = new Map<string, GrepMatch[]>();
    for (const hit of result?.matches ?? []) {
      const arr = m.get(hit.path);
      if (arr) arr.push(hit);
      else m.set(hit.path, [hit]);
    }
    return [...m.entries()];
  }, [result]);

  const toggle = (k: keyof typeof opts) => setOpts({ ...opts, [k]: !opts[k] });

  return (
    <>
      <div className="panel-header">
        <span className="title">Search</span>
        {busy && <div className="spinner" />}
      </div>
      <div className="search-box">
        <div className="search-input">
          <input ref={input} className="input" placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} data-testid="search-input" />
          <div className="search-toggles">
            <button className={opts.caseSensitive ? "on" : ""} title="Match case" onClick={() => toggle("caseSensitive")}>
              Aa
            </button>
            <button className={opts.wholeWord ? "on" : ""} title="Whole word" onClick={() => toggle("wholeWord")}>
              ab
            </button>
            <button className={opts.regex ? "on" : ""} title="Regular expression" onClick={() => toggle("regex")}>
              .*
            </button>
          </div>
        </div>
        <input className="input" placeholder="Files to include (e.g. *.rs, src/**)" value={include} onChange={(e) => setInclude(e.target.value)} />
        {result && (
          <div className="muted" style={{ fontSize: 12 }}>
            {result.matches.length}
            {result.truncated ? "+" : ""} result{result.matches.length === 1 ? "" : "s"} in {groups.length} file{groups.length === 1 ? "" : "s"}
          </div>
        )}
        {error && <div style={{ color: "var(--err)", fontSize: 12 }}>{error}</div>}
      </div>
      <div className="search-results">
        {groups.map(([path, hits]) => (
          <div key={path}>
            <div
              className="search-file"
              onClick={() =>
                setCollapsed((c) => {
                  const n = new Set(c);
                  if (n.has(path)) n.delete(path);
                  else n.add(path);
                  return n;
                })
              }
              title={path}
            >
              <Icon name={collapsed.has(path) ? "chevronRight" : "chevronDown"} size={12} />
              <FileBadge name={basename(path)} />
              <span className="ellipsis">
                {basename(path)} <span className="muted">{dirname(path)}</span>
              </span>
              <span className="count">{hits.length}</span>
            </div>
            {!collapsed.has(path) &&
              hits.map((h) => (
                <div key={h.line} className="search-hit" title={h.text} onClick={() => void useEditor.getState().openFile(h.path, h.line)}>
                  <HitText m={h} />
                </div>
              ))}
          </div>
        ))}
      </div>
    </>
  );
}
