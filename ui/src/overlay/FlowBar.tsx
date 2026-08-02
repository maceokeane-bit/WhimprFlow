import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { palette, pillFill, geometry, font } from "../tokens/values";

// Visual states, mirroring the Rust `BarState`.
export type BarState =
  | "idle"
  | "listening"
  | "recording"
  | "locked"
  | "transcribing"
  | "formatting"
  | "processing"
  | "done"
  | "paused"
  | "cancelled"
  | "error";

type StateEvent = { state: BarState; message?: string };
type WaveformEvent = { bars: number[] };
type OverlayBridgeWindow = Window &
  typeof globalThis & {
    __WHIMPR_OVERLAY_STATE__?: StateEvent;
    __WHIMPR_OVERLAY_WAVEFORM__?: WaveformEvent;
  };

async function tauriListen<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  return listen<T>(event, (e) => cb(e.payload));
}

function Spinner() {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 3, height: 20 }} aria-hidden="true">
      {[0, 1, 2].map((index) => (
        <span
          key={index}
          style={{
            width: 3,
            height: 13,
            borderRadius: 999,
            background: palette.accent400,
            boxShadow: `0 0 8px ${palette.accentGlow}`,
            animation: `whimpr-prepare 0.9s ease-in-out ${index * 0.12}s infinite`,
          }}
        />
      ))}
    </div>
  );
}

// A row of dot-like rounded bars driven by mic RMS — Wispr's dotted-waveform look:
// small dots when quiet, rising into a waveform when speaking.
function DottedWaveform({ bars }: { bars: number[] }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const barsRef = useRef<number[]>(bars);
  const displayedRef = useRef<number[]>(Array.from({ length: 16 }, () => 0));
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
        const target = Math.max(0, Math.min(1, barsRef.current[i] ?? 0));
        displayedRef.current[i] += (target - displayedRef.current[i]) * 0.28;
        const idleMotion = 0.08 + 0.04 * Math.abs(Math.sin(t / 260 + i * 0.7));
        const amp = Math.max(idleMotion, displayedRef.current[i]);
        const bh = 3 + amp * 22;
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
        background: palette.accent500,
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

  useEffect(() => {
    let un1: (() => void) | undefined;
    let un2: (() => void) | undefined;

    const applyState = (p: StateEvent) => {
      setState(p.state);
      setMessage(p.message);
    };
    const applyWaveform = (p: WaveformEvent) => setBars(p.bars);
    const onPanelState = (event: Event) =>
      applyState((event as CustomEvent<StateEvent>).detail);
    const onPanelWaveform = (event: Event) =>
      applyWaveform((event as CustomEvent<WaveformEvent>).detail);

    window.addEventListener("whimpr:overlay-state", onPanelState);
    window.addEventListener("whimpr:overlay-waveform", onPanelWaveform);
    tauriListen<StateEvent>("whimpr://flowbar/state", applyState).then((u) => (un1 = u));
    tauriListen<WaveformEvent>("whimpr://audio/waveform", applyWaveform).then(
      (u) => (un2 = u),
    );

    // Main-thread Rust injection also stores the latest payload globally. Polling
    // these references avoids relying on NSPanel event propagation altogether.
    const bridge = window as OverlayBridgeWindow;
    let lastState = bridge.__WHIMPR_OVERLAY_STATE__;
    let lastWaveform = bridge.__WHIMPR_OVERLAY_WAVEFORM__;
    const bridgePoll = window.setInterval(() => {
      if (bridge.__WHIMPR_OVERLAY_STATE__ !== lastState) {
        lastState = bridge.__WHIMPR_OVERLAY_STATE__;
        if (lastState) applyState(lastState);
      }
      if (bridge.__WHIMPR_OVERLAY_WAVEFORM__ !== lastWaveform) {
        lastWaveform = bridge.__WHIMPR_OVERLAY_WAVEFORM__;
        if (lastWaveform) applyWaveform(lastWaveform);
      }
    }, 16);

    return () => {
      window.clearInterval(bridgePoll);
      window.removeEventListener("whimpr:overlay-state", onPanelState);
      window.removeEventListener("whimpr:overlay-waveform", onPanelWaveform);
      un1?.();
      un2?.();
    };
  }, []);

  useEffect(() => {
    setSlow(false);
    if (!["transcribing", "formatting", "processing"].includes(state)) return;
    const id = window.setTimeout(() => setSlow(true), 4000);
    return () => window.clearTimeout(id);
  }, [state]);

  const recording = state === "recording" || state === "locked";
  const listening = state === "listening";
  const isIdle = state === "idle";
  const processing = ["transcribing", "formatting", "processing"].includes(state);
  const isError = state === "error";
  const statusText =
    message ??
    (slow && processing
      ? "Taking longer than usual"
      : state === "transcribing"
        ? "Transcribing…"
        : state === "formatting"
          ? "Using Polish"
          : state === "processing"
            ? "Preparing your text…"
            : state === "listening" || state === "recording"
              ? "Listening…"
              : state === "locked"
                ? "Hands-free"
                : state === "paused"
                  ? "Dictation paused"
                  : isError
                    ? "Something's not right"
                    : state === "cancelled"
                      ? "Discarded"
                      : state === "done"
                        ? "Done"
                        : "");

  const dims = isIdle
    ? { w: 76, h: 16 }
    : recording
      ? { w: 250, h: 44 }
      : listening
        ? { w: 150, h: 36 }
      : processing && slow
        ? { w: 230, h: 38 }
        : processing
          ? { w: 205, h: 38 }
        : isError
          ? { w: Math.min(300, Math.max(210, statusText.length * 7.2 + 48)), h: 38 }
          : { w: 180, h: 36 };

  const borderColor = isError
    ? "rgba(255,107,107,0.45)"
    : recording || listening
      ? "rgba(34,195,182,0.35)"
      : "rgba(255,255,255,0.10)";

  return (
    <>
      <style>{`
        @keyframes whimpr-prepare {
          0%, 100% { opacity: 0.35; transform: scaleY(0.45); }
          50% { opacity: 1; transform: scaleY(1); }
        }
        @keyframes whimpr-pulse {
          0%, 100% { opacity: 0.55; transform: scale(0.92); }
          50% { opacity: 1; transform: scale(1); }
        }
        @keyframes whimpr-content-in {
          from { opacity: 0; transform: scale(0.86) translateY(2px); }
          to { opacity: 1; transform: scale(1) translateY(0); }
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
          onContextMenu={(event) => {
            event.preventDefault();
            void invoke("show_flow_menu");
          }}
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: recording ? "space-between" : "center",
            gap: 10,
            height: dims.h,
            width: dims.w,
            padding: recording ? "0 8px" : processing || isError ? "0 16px" : 0,
            background: pillFill.base,
            border: `1px solid ${borderColor}`,
            borderRadius: 9999,
            boxShadow: isError ? "0 8px 28px rgba(255,90,82,0.22)" : pillFill.shadow,
            color: palette.pillText,
            transition: `width ${geometry.morphMs}ms ${motionEase}, height ${geometry.morphMs}ms ${motionEase}, padding ${geometry.morphMs}ms ${motionEase}, transform ${geometry.morphMs}ms ${motionEase}, border-color 240ms ease, box-shadow ${geometry.morphMs}ms ease`,
            overflow: "hidden",
            fontSize: 13,
            transform: isIdle ? "scale(0.96)" : "scale(1)",
            transformOrigin: "center",
          }}
        >
          <div
            key={state}
            style={{
              width: "100%",
              display: "flex",
              alignItems: "center",
              justifyContent: recording ? "space-between" : "center",
              gap: 10,
              animation: `whimpr-content-in ${Math.min(240, geometry.morphMs)}ms ${motionEase}`,
            }}
          >
            {isIdle ? (
              <IdleDot />
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
                {listening && <IdleDot />}
                {isError && <span aria-hidden="true">⚠</span>}
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
      </div>
    </>
  );
}

const motionEase = "cubic-bezier(0.05,0.6,0.4,0.95)";
