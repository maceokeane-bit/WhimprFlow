import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { palette, pillFill, geometry, font, motion } from "../tokens/values";

// Visual states, mirroring the Rust `BarState`.
export type BarState =
  | "idle"
  | "recording"
  | "locked"
  | "transcribing"
  | "done"
  | "cancelled"
  | "error";

type StateEvent = { state: BarState; message?: string };
type WaveformEvent = { bars: number[] };

async function tauriListen<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  return listen<T>(event, (e) => cb(e.payload));
}

function Spinner() {
  return (
    <div
      style={{
        width: 14,
        height: 14,
        borderRadius: "50%",
        border: "2px solid rgba(255,255,255,0.18)",
        borderTopColor: palette.accent,
        animation: "whimpr-spin 0.75s linear infinite",
        flex: "0 0 auto",
      }}
    />
  );
}

// A row of dot-like rounded bars driven by mic RMS — Wispr's dotted-waveform look:
// small dots when quiet, rising into a waveform when speaking.
function DottedWaveform({ bars }: { bars: number[] }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const barsRef = useRef<number[]>(bars);
  barsRef.current = bars;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    let raf = 0;
    const N = 16;
    const draw = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
        canvas.width = w * dpr;
        canvas.height = h * dpr;
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const dotW = 2.4;
      const gap = (w - N * dotW) / (N - 1);
      const t = performance.now();
      ctx.fillStyle = palette.waveBar;
      for (let i = 0; i < N; i++) {
        const real = barsRef.current[barsRef.current.length - 1 - (i % barsRef.current.length)];
        const shimmer = 0.12 + 0.06 * Math.abs(Math.sin(t / 260 + i * 0.7));
        const amp = Math.max(shimmer, real ?? 0);
        const bh = 3 + amp * 20;
        const x = i * (dotW + gap);
        const y = (h - bh) / 2;
        ctx.beginPath();
        ctx.roundRect(x, y, dotW, bh, dotW / 2);
        ctx.fill();
      }
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, []);

  return <canvas ref={canvasRef} style={{ width: "100%", height: 28 }} />;
}

function CancelButton() {
  return (
    <div
      title="Cancel (Esc)"
      style={{
        flex: "0 0 auto",
        width: 26,
        height: 26,
        borderRadius: 9999,
        background: "rgba(255,255,255,0.16)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        color: "#fff",
        fontSize: 15,
        lineHeight: 1,
      }}
    >
      ✕
    </div>
  );
}

function StopButton() {
  return (
    <div
      title="Stop"
      style={{
        flex: "0 0 auto",
        width: 26,
        height: 26,
        borderRadius: 9999,
        background: "#FF5A52",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <div style={{ width: 9, height: 9, borderRadius: 2, background: "#fff" }} />
    </div>
  );
}

function IdleDot() {
  return (
    <div
      style={{
        width: 8,
        height: 8,
        borderRadius: "50%",
        background: palette.accent,
        boxShadow: `0 0 10px ${palette.accentGlow}`,
        opacity: 0.85,
        animation: "whimpr-pulse 2.4s ease-in-out infinite",
      }}
    />
  );
}

export function FlowBar() {
  const [state, setState] = useState<BarState>("idle");
  const [message, setMessage] = useState<string | undefined>();
  const [bars, setBars] = useState<number[]>([]);
  const [slow, setSlow] = useState(false);
  const transcribingSince = useRef<number | null>(null);

  useEffect(() => {
    let un1: (() => void) | undefined;
    let un2: (() => void) | undefined;
    tauriListen<StateEvent>("whimpr://flowbar/state", (p) => {
      setState(p.state);
      setMessage(p.message);
      if (p.state === "transcribing") {
        transcribingSince.current = Date.now();
        setSlow(false);
      } else {
        transcribingSince.current = null;
        setSlow(false);
      }
    }).then((u) => (un1 = u));
    tauriListen<WaveformEvent>("whimpr://audio/waveform", (p) => setBars(p.bars)).then((u) => (un2 = u));
    return () => {
      un1?.();
      un2?.();
    };
  }, []);

  useEffect(() => {
    if (state !== "transcribing") return;
    const id = window.setTimeout(() => setSlow(true), 4000);
    return () => window.clearTimeout(id);
  }, [state]);

  const recording = state === "recording" || state === "locked";
  const isIdle = state === "idle";
  const processing = state === "transcribing";
  const isError = state === "error";
  const statusText =
    message ??
    (processing
      ? slow
        ? "Taking longer than usual…"
        : "Cleaning up…"
      : isError
        ? "Something's off"
        : state === "cancelled"
          ? "Discarded"
          : state === "done"
            ? "Done"
            : "");

  const dims = isIdle
    ? { w: 120, h: 32 }
    : recording
      ? { w: 250, h: 44 }
      : processing && slow
        ? { w: 220, h: 36 }
        : isError
          ? { w: 210, h: 36 }
          : { w: 180, h: 36 };

  const borderColor = isError
    ? "rgba(255,107,107,0.45)"
    : recording
      ? "rgba(34,195,182,0.35)"
      : "rgba(255,255,255,0.10)";

  return (
    <>
      <style>{`
        @keyframes whimpr-spin { to { transform: rotate(360deg); } }
        @keyframes whimpr-pulse {
          0%, 100% { opacity: 0.55; transform: scale(0.92); }
          50% { opacity: 1; transform: scale(1); }
        }
      `}</style>
      <div
        style={{
          position: "fixed",
          inset: 0,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontFamily: font.ui,
          userSelect: "none",
        }}
      >
        <div
          aria-label={`WhimprFlow ${state}`}
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: recording ? "space-between" : "center",
            gap: isIdle ? 6 : 10,
            height: dims.h,
            width: dims.w,
            padding: recording ? "0 8px" : processing || isError ? "0 14px" : isIdle ? "0 10px" : 0,
            background: pillFill.base,
            border: `1px solid ${borderColor}`,
            borderRadius: 9999,
            boxShadow: isError ? "0 8px 28px rgba(255,90,82,0.22)" : pillFill.shadow,
            color: palette.pillText,
            transition: `width ${geometry.morphMs}ms ${motionEase}, height ${geometry.morphMs}ms ${motionEase}, border-color 240ms ease`,
            overflow: "hidden",
            fontSize: 13,
          }}
        >
          {isIdle ? (
            <>
              <IdleDot />
              <span style={{ fontSize: 11, color: palette.pillTextMuted, marginLeft: 6 }}>
                Whimpr
              </span>
            </>
          ) : recording ? (
            <>
              <CancelButton />
              <div style={{ flex: 1, minWidth: 0 }}>
                <DottedWaveform bars={bars} />
              </div>
              <StopButton />
            </>
          ) : (
            <>
              {processing && <Spinner />}
              <span
                style={{
                  color: isError ? "#FFB4B4" : palette.pillTextMuted,
                  whiteSpace: "nowrap",
                }}
              >
                {statusText}
              </span>
            </>
          )}
        </div>
      </div>
    </>
  );
}

const motionEase = motion.ease;
