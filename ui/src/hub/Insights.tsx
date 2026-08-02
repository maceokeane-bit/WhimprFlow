import { useEffect, useState } from "react";
import { font } from "../tokens/values";
import { theme } from "./theme";
import { Button, Card, PageTitle, useStats } from "./ui";
import { analyzeInsights, getLanguageStats, type InsightReport, type LanguageStats, type StatsSummary } from "./api";
import { fmtCompact, fmtNum, newsArticles } from "./format";

// ── Semicircular gauge ───────────────────────────────────────────────────────
function Gauge({ value, max }: { value: number; max: number }) {
  const frac = Math.max(0, Math.min(1, value / max));
  const r = 58;
  const cx = 80;
  const cy = 72;
  const len = Math.PI * r;
  const d = `M ${cx - r} ${cy} A ${r} ${r} 0 0 1 ${cx + r} ${cy}`;
  return (
    <div style={{ position: "relative", width: 160, height: 88, margin: "0 auto" }}>
      <svg width="160" height="88" viewBox="0 0 160 88">
        <path d={d} fill="none" stroke={theme.track} strokeWidth="12" strokeLinecap="round" />
        <path
          d={d}
          fill="none"
          stroke={theme.accent}
          strokeWidth="12"
          strokeLinecap="round"
          strokeDasharray={len}
          strokeDashoffset={len * (1 - frac)}
        />
      </svg>
      <div
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 2,
          textAlign: "center",
        }}
      >
        <div style={{ fontFamily: font.serif, fontSize: 34, fontWeight: 600, color: theme.textStrong, lineHeight: 1 }}>
          {fmtNum(value)}
        </div>
      </div>
    </div>
  );
}

function StatCard({
  label,
  children,
  foot,
}: {
  label: string;
  children: React.ReactNode;
  foot?: React.ReactNode;
}) {
  return (
    <Card style={{ flex: "1 1 200px", minWidth: 0 }}>
      <div
        style={{
          fontSize: 11.5,
          fontWeight: 700,
          letterSpacing: 0.6,
          textTransform: "uppercase",
          color: theme.textFaint,
          marginBottom: 14,
        }}
      >
        {label}
      </div>
      {children}
      {foot && <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 12, textAlign: "center" }}>{foot}</div>}
    </Card>
  );
}

function BigNumber({ value, accent }: { value: string; accent?: boolean }) {
  return (
    <div
      style={{
        fontFamily: font.serif,
        fontSize: 44,
        fontWeight: 600,
        lineHeight: 1,
        textAlign: "center",
        color: accent ? theme.accentDeep : theme.textStrong,
      }}
    >
      {value}
    </div>
  );
}

// ── 7-day bar chart ──────────────────────────────────────────────────────────
const DOW = ["S", "M", "T", "W", "T", "F", "S"];

function ActivityBars({ data }: { data: number[] }) {
  const max = Math.max(1, ...data);
  const todayIdx = new Date().getDay(); // 0..6, last bar = today
  return (
    <div>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 8, height: 120 }}>
        {data.map((v, i) => (
          <div key={i} style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "flex-end", height: "100%" }}>
            <div
              title={`${fmtNum(v)} words`}
              style={{
                height: `${v > 0 ? Math.max(6, (v / max) * 100) : 3}%`,
                background: v > 0 ? theme.accent : theme.track,
                borderRadius: 6,
                transition: "height 240ms ease",
              }}
            />
          </div>
        ))}
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
        {data.map((_, i) => {
          // Map the 7 bars onto weekday initials ending at today.
          const dow = (todayIdx - (data.length - 1 - i) + 7) % 7;
          return (
            <div key={i} style={{ flex: 1, textAlign: "center", fontSize: 10.5, color: theme.textFaint }}>
              {i === data.length - 1 ? "Today" : DOW[dow]}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Contribution heatmap (illustrative) ──────────────────────────────────────
const HEAT_WEEKS = 12;

function level(v: number, max: number): number {
  if (v <= 0) return 0;
  const r = v / max;
  if (r < 0.25) return 1;
  if (r < 0.5) return 2;
  if (r < 0.75) return 3;
  return 4;
}

const HEAT_COLORS = [theme.track, "rgba(34,195,182,0.28)", "rgba(34,195,182,0.5)", "rgba(34,195,182,0.72)", theme.accentDeep];

function Heatmap({ last7 }: { last7: number[] }) {
  const max = Math.max(1, ...last7);
  const cols: number[][] = [];
  for (let w = 0; w < HEAT_WEEKS; w++) {
    const col: number[] = [];
    for (let day = 0; day < 7; day++) {
      // Only the most-recent week (rightmost column) carries real data.
      col.push(w === HEAT_WEEKS - 1 ? (last7[day] ?? 0) : 0);
    }
    cols.push(col);
  }
  return (
    <div style={{ display: "flex", gap: 4, overflowX: "auto" }}>
      {cols.map((col, w) => (
        <div key={w} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {col.map((v, day) => (
            <div
              key={day}
              title={v > 0 ? `${fmtNum(v)} words` : "no activity"}
              style={{
                width: 13,
                height: 13,
                borderRadius: 3.5,
                background: HEAT_COLORS[level(v, max)],
              }}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

// ── Tabs ─────────────────────────────────────────────────────────────────────
type Tab = "usage" | "voice";

function Tabs({ tab, onChange }: { tab: Tab; onChange: (t: Tab) => void }) {
  const items: { key: Tab; label: string }[] = [
    { key: "usage", label: "Your Usage" },
    { key: "voice", label: "Your Voice" },
  ];
  return (
    <div style={{ display: "flex", gap: 24, borderBottom: `1px solid ${theme.border}`, marginBottom: 22 }}>
      {items.map((it) => {
        const active = tab === it.key;
        return (
          <button
            key={it.key}
            onClick={() => onChange(it.key)}
            style={{
              border: "none",
              background: "transparent",
              cursor: "pointer",
              fontFamily: font.ui,
              fontSize: 14,
              fontWeight: active ? 600 : 500,
              color: active ? theme.textStrong : theme.textMuted,
              padding: "0 0 12px",
              marginBottom: -1,
              borderBottom: `2px solid ${active ? theme.accent : "transparent"}`,
            }}
          >
            {it.label}
          </button>
        );
      })}
    </div>
  );
}

function UsageTab({ stats }: { stats: StatsSummary }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 18 }}>
      {/* Top row — three stat cards */}
      <div style={{ display: "flex", flexWrap: "wrap", gap: 18 }}>
        <StatCard label="Words per minute" foot="Top 5% of dictators">
          <Gauge value={stats.avg_wpm} max={140} />
        </StatCard>

        <StatCard label="Dictations cleaned" foot="sessions with cleanup applied">
          <BigNumber value={fmtCompact(stats.total_sessions)} accent />
        </StatCard>

        <StatCard label="Total words dictated" foot={`≈ ${fmtNum(newsArticles(stats.total_words))} news articles`}>
          <BigNumber value={fmtCompact(stats.total_words)} />
        </StatCard>
      </div>

      {/* Bottom row — activity + streak */}
      <div style={{ display: "flex", flexWrap: "wrap", gap: 18 }}>
        <Card style={{ flex: "1 1 340px", minWidth: 0 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 16 }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>7-day activity</div>
            <div style={{ fontSize: 12, color: theme.textFaint }}>{fmtNum(stats.words_today)} today</div>
          </div>
          <ActivityBars data={stats.last7_words} />
        </Card>

        <Card style={{ flex: "1 1 300px", minWidth: 0 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: 16 }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>Streak</div>
            <div style={{ fontSize: 13, fontWeight: 600, color: theme.accentDeep }}>
              🔥 {stats.day_streak} {stats.day_streak === 1 ? "day" : "days"}
            </div>
          </div>
          <Heatmap last7={stats.last7_words} />
          <div style={{ fontSize: 12, color: theme.textFaint, marginTop: 14 }}>
            Each square is a day. Keep the streak alive by dictating something every day.
          </div>
        </Card>
      </div>
    </div>
  );
}

function LocalStatsPanel({ stats }: { stats: LanguageStats }) {
  if (stats.sessions_analyzed === 0) return null;
  const metrics = [
    { label: "Avg words / dictation", value: stats.avg_words_per_session.toFixed(1) },
    { label: "Speaking pace", value: `${stats.avg_wpm} WPM` },
    { label: "Avg sentence length", value: stats.avg_sentence_length.toFixed(1) },
    { label: "Filler words", value: `${stats.filler_per_100_words.toFixed(1)} / 100 words` },
    { label: "Cleanup edits", value: `${Math.round(stats.cleanup_edit_rate * 100)}% of sessions` },
    { label: "Vocabulary breadth", value: `${Math.round(stats.unique_word_ratio * 100)}% unique` },
  ];
  return (
    <Card>
      <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong, marginBottom: 14 }}>
        Local analysis ({stats.sessions_analyzed} recent dictations)
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(180px, 1fr))", gap: 14 }}>
        {metrics.map((m) => (
          <div key={m.label}>
            <div style={{ fontSize: 11, color: theme.textFaint, textTransform: "uppercase", letterSpacing: 0.5 }}>
              {m.label}
            </div>
            <div style={{ fontSize: 18, fontWeight: 600, color: theme.textStrong, marginTop: 4 }}>{m.value}</div>
          </div>
        ))}
      </div>
      {stats.top_apps.length > 0 && (
        <div style={{ marginTop: 16, fontSize: 12.5, color: theme.textMuted }}>
          Most dictated in: {stats.top_apps.map(([a, n]) => `${a} (${n})`).join(", ")}
        </div>
      )}
    </Card>
  );
}

function VoiceTab() {
  const [report, setReport] = useState<InsightReport | null>(null);
  const [local, setLocal] = useState<LanguageStats | null>(null);
  const [loading, setLoading] = useState(false);

  const load = async (force = false) => {
    setLoading(true);
    const [r, l] = await Promise.all([analyzeInsights(force), getLanguageStats()]);
    setReport(r);
    setLocal(l);
    setLoading(false);
  };

  useEffect(() => {
    void load(false);
  }, []);

  if (loading && !report) {
    return (
      <Card>
        <div style={{ padding: "36px 8px", textAlign: "center", color: theme.textMuted, fontSize: 14 }}>
          Analyzing your recent dictations…
        </div>
      </Card>
    );
  }

  if (!report || report.error) {
    return (
      <Card>
        <div style={{ padding: "28px 8px", textAlign: "center" }}>
          <div style={{ fontFamily: font.serif, fontSize: 20, fontWeight: 600, color: theme.textStrong }}>
            Your Voice
          </div>
          <p style={{ color: theme.textMuted, fontSize: 14, lineHeight: 1.55, maxWidth: 460, margin: "10px auto 0" }}>
            {report?.error ??
              "Dictate a few times, then come back — WhimprFlow will estimate reading level, complexity, and topics via Ollama."}
          </p>
          <div style={{ marginTop: 16 }}>
            <Button onClick={() => void load(true)} disabled={loading}>
              {loading ? "Analyzing…" : "Analyze now"}
            </Button>
          </div>
        </div>
      </Card>
    );
  }

  const metrics = [
    { label: "Reading level", value: report.reading_grade },
    { label: "Complexity", value: report.complexity },
    { label: "Domain depth", value: report.domain_depth },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 18 }}>
      {local && <LocalStatsPanel stats={local} />}

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div style={{ fontSize: 13, color: theme.textMuted }}>
          Based on {report.sessions_analyzed} recent dictation{report.sessions_analyzed === 1 ? "" : "s"}
        </div>
        <Button variant="ghost" size="sm" onClick={() => void load(true)} disabled={loading}>
          Refresh
        </Button>
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: 18 }}>
        {metrics.map((m) => (
          <Card key={m.label} style={{ flex: "1 1 180px", minWidth: 0 }}>
            <div style={{ fontSize: 11.5, fontWeight: 700, letterSpacing: 0.6, textTransform: "uppercase", color: theme.textFaint }}>
              {m.label}
            </div>
            <div style={{ fontFamily: font.serif, fontSize: 28, fontWeight: 600, color: theme.textStrong, marginTop: 8 }}>
              {m.value || "—"}
            </div>
          </Card>
        ))}
      </div>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong, marginBottom: 8 }}>Summary</div>
        <p style={{ color: theme.textBody, fontSize: 14, lineHeight: 1.6, margin: 0 }}>{report.summary}</p>
        {report.vocabulary_note && (
          <p style={{ color: theme.textMuted, fontSize: 13, lineHeight: 1.55, margin: "12px 0 0" }}>
            {report.vocabulary_note}
          </p>
        )}
      </Card>

      {report.topics.length > 0 && (
        <Card>
          <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong, marginBottom: 12 }}>Main topics</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
            {report.topics.map((t) => (
              <span
                key={t}
                style={{
                  fontSize: 12.5,
                  padding: "6px 12px",
                  borderRadius: 9999,
                  background: theme.accentSoft,
                  border: `1px solid ${theme.accentSoftBorder}`,
                  color: theme.accentDeep,
                }}
              >
                {t}
              </span>
            ))}
          </div>
        </Card>
      )}
    </div>
  );
}

export function Insights() {
  const stats = useStats();
  const [tab, setTab] = useState<Tab>("usage");
  return (
    <div style={{ maxWidth: 1000 }}>
      <PageTitle>Insights</PageTitle>
      <Tabs tab={tab} onChange={setTab} />
      {tab === "usage" ? <UsageTab stats={stats} /> : <VoiceTab />}
    </div>
  );
}
