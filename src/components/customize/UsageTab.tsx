import { useEffect, useState } from "react";
import { Icon } from "../Icon";
import { useApp } from "../../store/app";
import { api, errorText } from "../../lib/ipc";
import { formatTokens } from "../../lib/util";
import { PROVIDER_LABEL } from "../../lib/models";
import type { Provider, UsageRow, UsageSummary } from "../../lib/types";

const RANGES = [7, 30, 90];

function Table({ rows, label, name }: { rows: UsageRow[]; label: string; name: (k: string) => string }) {
  if (!rows.length) return null;
  return (
    <table className="data-table">
      <thead>
        <tr>
          <th>{label}</th>
          <th className="num">Requests</th>
          <th className="num">Input</th>
          <th className="num">Cached</th>
          <th className="num">Output</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <td>{name(r.key) || "—"}</td>
            <td className="num">{r.requests.toLocaleString()}</td>
            <td className="num">{formatTokens(r.inputTokens + r.cacheWriteTokens)}</td>
            <td className="num">{formatTokens(r.cacheReadTokens)}</td>
            <td className="num">{formatTokens(r.outputTokens)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function UsageTab() {
  const [days, setDays] = useState(30);
  const [u, setU] = useState<UsageSummary | null>(null);
  const toast = useApp.getState().toast;
  const load = () => api.usageSummary(days).then(setU, (e) => toast(errorText(e), "error"));
  useEffect(() => void load(), [days]);

  const inTokens = (r: UsageRow) => r.inputTokens + r.cacheReadTokens + r.cacheWriteTokens;
  const peak = Math.max(1, ...(u?.byDay ?? []).map((d) => inTokens(d) + d.outputTokens));
  const period = u?.byDay.reduce((a, d) => ({ i: a.i + inTokens(d), o: a.o + d.outputTokens, r: a.r + d.requests }), { i: 0, o: 0, r: 0 });

  return (
    <>
      <h3>Usage</h3>
      <p className="lead">Tokens used by every model request, across all providers. Recorded on this machine only; days are in UTC.</p>
      <div className="hub-actions">
        <div className="segmented">
          {RANGES.map((d) => (
            <button key={d} className={d === days ? "on" : ""} onClick={() => setDays(d)}>
              {d} days
            </button>
          ))}
        </div>
        <span style={{ flex: 1 }} />
        <button className="btn ghost sm" onClick={() => void load()}>
          <Icon name="refresh" size={13} /> Refresh
        </button>
        <button
          className="btn ghost sm"
          onClick={() => confirm("Delete all usage records?") && void api.clearUsage().then(load, (e) => toast(errorText(e), "error"))}
        >
          <Icon name="trash" size={13} /> Clear
        </button>
      </div>
      {u && (
        <>
          <div className="stat-cards" data-testid="usage-cards">
            <div className="stat-card">
              <div className="k">Today</div>
              <div className="v">{formatTokens(inTokens(u.today) + u.today.outputTokens)}</div>
            </div>
            <div className="stat-card">
              <div className="k">Last {days} days</div>
              <div className="v">{formatTokens((period?.i ?? 0) + (period?.o ?? 0))}</div>
            </div>
            <div className="stat-card">
              <div className="k">Requests ({days}d)</div>
              <div className="v">{(period?.r ?? 0).toLocaleString()}</div>
            </div>
            <div className="stat-card">
              <div className="k">Chats (all time)</div>
              <div className="v">{u.conversations.toLocaleString()}</div>
            </div>
          </div>
          <div className="legend">
            <span>
              <i style={{ background: "var(--accent)" }} />
              Input (incl. cache)
            </span>
            <span>
              <i style={{ background: "var(--ok)" }} />
              Output
            </span>
          </div>
          <div className="bars">
            {u.byDay.map((d) => {
              const total = inTokens(d) + d.outputTokens;
              return (
                <div
                  key={d.key}
                  className="bar"
                  style={{ height: `${Math.max(2, (total / peak) * 100)}%` }}
                  title={`${d.key}: ${formatTokens(inTokens(d))} in · ${formatTokens(d.outputTokens)} out · ${d.requests} requests`}
                >
                  {total > 0 && (
                    <>
                      <div className="in" style={{ height: `${(inTokens(d) / total) * 100}%` }} />
                      <div className="out" style={{ height: `${(d.outputTokens / total) * 100}%` }} />
                    </>
                  )}
                </div>
              );
            })}
          </div>
          <div className="bars-axis">
            <span>{u.byDay[0]?.key}</span>
            <span>{u.byDay[u.byDay.length - 1]?.key}</span>
          </div>
          <Table rows={u.byModel} label="Model (all time)" name={(k) => k} />
          <Table rows={u.byProvider} label="Provider (all time)" name={(k) => PROVIDER_LABEL[k as Provider] ?? k} />
          {u.total.requests === 0 && <div className="hub-empty">No requests recorded yet.</div>}
        </>
      )}
    </>
  );
}
