import { useEffect, useRef, useState, useSyncExternalStore } from "react";
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

type StateEvent = { state: BarState; message?: string | null; epoch?: number };
type WaveformEvent = { bars: number[] };
type PartialEvent = { text: string };
type FlowBarSnap = {
  state: BarState;
  message?: string | null;
  bars: number[];
  preview: string;
  epoch: number;
};

type OverlayStore = {
  state: BarState;
  message?: string;
  bars: number[];
  preview: string;
  epoch: number;
};

const DEFAULT_BARS = () => Array.from({ length: 16 }, () => 0);

let store: OverlayStore = {
  state: "idle",
  message: undefined,
  bars: DEFAULT_BARS(),
  preview: "",
  epoch: 0,
};
const subscribers = new Set<() => void>();

function emitStore() {
  for (const sub of subscribers) sub();
}

function applyStore( partial: Partial<OverlayStore>, opts?: { force?: boolean }) {
  const nextEpoch = partial.epoch;
  if (
    !opts?.force &&
    typeof nextEpoch === "number" &&
    nextEpoch < store.epoch
  ) {
    return;
  }
  store = {
    ...store,
    ...partial,
    bars: partial.bars ?? store.bars,
    epoch:
      typeof nextEpoch === "number"
        ? Math.max(store.epoch, nextEpoch)
        : store.epoch,
  };
  emitStore();
}

function subscribeStore(onChange: () => void) {
  subscribers.add(onChange);
  return () => {
    subscribers.delete(onChange);
  };
}

function getStoreSnapshot() {
  return store;
}

function applyState(p: StateEvent) {
  applyStore(
    {
      state: p.state,
      message: p.message ?? undefined,
      preview: isRecordingState(p.state) ? store.preview : "",
      bars: isRecordingState(p.state) ? store.bars : DEFAULT_BARS(),
      epoch: p.epoch,
    },
    { force: typeof p.epoch !== "number" },
  );
}

function applyWaveform(p: WaveformEvent) {
  if (!Array.isArray(p.bars) || !p.bars.length) return;
  const promote =
    store.state === "idle" ||
    store.state === "done" ||
    store.state === "cancelled";
  applyStore(
    {
      bars: p.bars,
      state: promote ? "recording" : store.state,
      message: promote ? undefined : store.message,
    },
    { force: true },
  );
}

function applyPartial(p: PartialEvent) {
  const text = p.text?.trim();
  if (!text) return;
  applyStore(
    {
      preview: text,
      state: store.state === "idle" ? "recording" : store.state,
    },
    { force: true },
  );
}

function applySnap(snap: FlowBarSnap) {
  if (!snap || typeof snap !== "object") return;
  applyStore({
    state: snap.state || store.state,
    message: snap.message ?? undefined,
    bars: Array.isArray(snap.bars) && snap.bars.length ? snap.bars : store.bars,
    preview: typeof snap.preview === "string" ? snap.preview : store.preview,
    epoch: typeof snap.epoch === "number" ? snap.epoch : store.epoch,
  });
}

// Imperative bridge for Rust `window.eval` — registered before React mounts.
declare global {
  interface Window {
    __whimprApply?: (kind: string, payload: unknown) => void;
    __WHIMPR_OVERLAY_STATE__?: StateEvent;
    __WHIMPR_OVERLAY_WAVEFORM__?: WaveformEvent;
    __WHIMPR_OVERLAY_PARTIAL__?: PartialEvent;
  }
}

window.__whimprApply = (kind, payload) => {
  try {
    if (kind === "state") applyState(payload as StateEvent);
    else if (kind === "waveform") applyWaveform(payload as WaveformEvent);
    else if (kind === "partial") applyPartial(payload as PartialEvent);
    else if (kind === "snap") applySnap(payload as FlowBarSnap);
  } catch (error) {
    console.error("[whimpr] overlay apply failed", error);
  }
};

// Drain anything Rust pushed before this module evaluated.
if (window.__WHIMPR_OVERLAY_STATE__) applyState(window.__WHIMPR_OVERLAY_STATE__);
if (window.__WHIMPR_OVERLAY_WAVEFORM__) {
  applyWaveform(window.__WHIMPR_OVERLAY_WAVEFORM__);
}
if (window.__WHIMPR_OVERLAY_PARTIAL__) {
  applyPartial(window.__WHIMPR_OVERLAY_PARTIAL__);
}

function Spinner() {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 3, height: 18 }} aria-hidden="true">
      {[0, 1, 2].map((index) => (
        <span
          key={index}
          style={{
            width: 3,
            height: 12,
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
      if (w <= 0 || h <= 0) {
        raf = requestAnimationFrame(draw);
        return;
      }
      if (canvas.width !== Math.floor(w * dpr) || canvas.height !== Math.floor(h * dpr)) {
        canvas.width = Math.floor(w * dpr);
        canvas.height = Math.floor(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const dotW = 3;
      const gap = Math.max(2, (w - N * dotW) / (N - 1));
      const t = performance.now();
      ctx.fillStyle = palette.waveBar;
      for (let i = 0; i < N; i++) {
        const target = Math.max(0, Math.min(1, barsRef.current[i] ?? 0));
        displayedRef.current[i] += (target - displayedRef.current[i]) * 0.35;
        const idleMotion = 0.1 + 0.05 * Math.abs(Math.sin(t / 220 + i * 0.65));
        const amp = Math.max(idleMotion, displayedRef.current[i]);
        const bh = 4 + amp * (h - 8);
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

  return <canvas ref={canvasRef} style={{ width: "100%", height: 26, display: "block" }} />;
}

function SideButton({
  title,
  bg,
  children,
}: {
  title: string;
  bg: string;
  children: React.ReactNode;
}) {
  return (
    <div
      title={title}
      style={{
        flex: "0 0 auto",
        width: 26,
        height: 26,
        borderRadius: 9999,
        background: bg,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        color: "#fff",
        fontSize: 14,
        lineHeight: 1,
      }}
    >
      {children}
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
        opacity: 0.9,
        animation: "whimpr-pulse 2.4s ease-in-out infinite",
      }}
    />
  );
}

function isRecordingState(state: BarState) {
  return state === "recording" || state === "locked" || state === "listening";
}

function isProcessingState(state: BarState) {
  return state === "transcribing" || state === "formatting" || state === "processing";
}

function startOverlayBridge() {
  const onPanelState = (event: Event) =>
    applyState((event as CustomEvent<StateEvent>).detail);
  const onPanelWaveform = (event: Event) =>
    applyWaveform((event as CustomEvent<WaveformEvent>).detail);
  const onPanelPartial = (event: Event) =>
    applyPartial((event as CustomEvent<PartialEvent>).detail);

  window.addEventListener("whimpr:overlay-state", onPanelState);
  window.addEventListener("whimpr:overlay-waveform", onPanelWaveform);
  window.addEventListener("whimpr:overlay-partial", onPanelPartial);

  let un1: (() => void) | undefined;
  let un2: (() => void) | undefined;
  let un3: (() => void) | undefined;
  void listen<StateEvent>("whimpr://flowbar/state", (e) => applyState(e.payload)).then(
    (u) => (un1 = u),
  );
  void listen<WaveformEvent>("whimpr://audio/waveform", (e) => applyWaveform(e.payload)).then(
    (u) => (un2 = u),
  );
  void listen<PartialEvent>("whimpr://flowbar/partial", (e) => applyPartial(e.payload)).then(
    (u) => (un3 = u),
  );

  // Slow single-flight poll — backup only. A 33ms poll was wedging the overlay IPC
  // (CSS idle pulse kept running while React froze on "idle").
  let inFlight = false;
  const poll = window.setInterval(() => {
    if (inFlight) return;
    inFlight = true;
    void invoke<FlowBarSnap>("get_flow_bar_snap")
      .then(applySnap)
      .catch((error) => {
        console.warn("[whimpr] get_flow_bar_snap failed", error);
      })
      .finally(() => {
        inFlight = false;
      });
  }, 200);

  return () => {
    window.clearInterval(poll);
    window.removeEventListener("whimpr:overlay-state", onPanelState);
    window.removeEventListener("whimpr:overlay-waveform", onPanelWaveform);
    window.removeEventListener("whimpr:overlay-partial", onPanelPartial);
    un1?.();
    un2?.();
    un3?.();
  };
}

let bridgeStarted = false;
function ensureBridge() {
  if (bridgeStarted) return;
  bridgeStarted = true;
  startOverlayBridge();
}

export function FlowBar() {
  ensureBridge();
  const snap = useSyncExternalStore(subscribeStore, getStoreSnapshot, getStoreSnapshot);
  const { state, message, bars, preview } = snap;

  const recording = isRecordingState(state);
  const processing = isProcessingState(state);
  const isIdle = state === "idle";
  const isError = state === "error";

  const statusText =
    message?.trim() ||
    (state === "transcribing" || state === "formatting" || state === "processing"
      ? "Transcribing…"
      : state === "locked"
        ? "Hands-free"
        : state === "paused"
          ? "Paused"
          : isError
            ? "Something's not right"
            : state === "cancelled"
              ? "Discarded"
              : state === "done"
                ? "Done"
                : "");

  const previewText =
    preview.length > 64 ? `${preview.slice(0, 61).trimEnd()}…` : preview;

  const dims = isIdle
    ? { w: 72, h: 18 }
    : recording && previewText
      ? { w: 340, h: 58 }
      : recording
        ? { w: 260, h: 44 }
        : processing
          ? { w: 168, h: 36 }
          : isError
            ? { w: Math.min(280, Math.max(180, statusText.length * 7 + 40)), h: 36 }
            : { w: 140, h: 34 };

  const borderColor = isError
    ? "rgba(255,107,107,0.45)"
    : recording || processing
      ? "rgba(34,195,182,0.4)"
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
            padding: recording ? "0 8px" : processing || isError ? "0 14px" : 0,
            background: pillFill.base,
            border: `1px solid ${borderColor}`,
            borderRadius: 9999,
            boxShadow: isError ? "0 8px 28px rgba(255,90,82,0.22)" : pillFill.shadow,
            color: palette.pillText,
            transition: `width ${geometry.morphMs}ms ${motionEase}, height ${geometry.morphMs}ms ${motionEase}, padding ${geometry.morphMs}ms ${motionEase}, border-color 200ms ease`,
            overflow: "hidden",
            fontSize: 13,
          }}
        >
          {isIdle ? (
            <IdleDot />
          ) : recording ? (
            <>
              <SideButton title="Cancel (Esc)" bg="rgba(255,255,255,0.16)">
                ✕
              </SideButton>
              <div style={{ flex: 1, minWidth: 0 }}>
                <DottedWaveform bars={bars} />
                {previewText ? (
                  <div
                    style={{
                      marginTop: 3,
                      fontSize: 11,
                      lineHeight: 1.2,
                      color: palette.pillTextMuted,
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {previewText}
                  </div>
                ) : null}
              </div>
              <SideButton title="Stop" bg="#FF5A52">
                <div style={{ width: 9, height: 9, borderRadius: 2, background: "#fff" }} />
              </SideButton>
            </>
          ) : (
            <>
              {(processing || state === "done") && <Spinner />}
              {isError && <span aria-hidden="true">⚠</span>}
              <span
                style={{
                  color: isError ? "#FFB4B4" : palette.pillTextMuted,
                  whiteSpace: "nowrap",
                  fontWeight: 500,
                }}
              >
                {statusText || "Working…"}
              </span>
            </>
          )}
        </div>
      </div>
    </>
  );
}

const motionEase = "cubic-bezier(0.05,0.6,0.4,0.95)";
