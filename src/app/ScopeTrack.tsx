import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import type { Annotation, DisplayFormat, Envelope } from "./App";

export type TimeView = { start: number; span: number };
export type VoltageView = { center: number; division: number };
export const COLORS = ["#171717", "#525252"];
export function timeLabel(t: number) {
  const abs = Math.abs(t);
  return abs < 0.001 ? `${(t * 1e6).toFixed(1)} µs` : abs < 1 ? `${(t * 1e3).toFixed(2)} ms` : `${t.toFixed(3)} s`;
}
export function fitVoltage(values: number[]): VoltageView {
  if (!values.length) return { center: 0, division: 1 };
  let min = Infinity, max = -Infinity;
  for (const value of values) { min = Math.min(min, value); max = Math.max(max, value); }
  return { center: (min + max) / 2, division: Math.max(0.001, (max - min) / 6) };
}
export function limitTime(view: TimeView, duration: number, rate: number): TimeView {
  const span = Math.max(Math.min(8 / rate, duration), Math.min(duration, view.span));
  return { span, start: Math.max(0, Math.min(duration - span, view.start)) };
}

type Props = {
  id: number; envelope: Envelope; sampleCount: number; rate: number; time: TimeView; voltage: VoltageView;
  mode: "pan" | "window"; visible: boolean;
  annotations: Annotation[]; format: DisplayFormat; threshold: number;
  onTime: (view: TimeView) => void; onVoltage: (view: VoltageView) => void;
};
const LEFT = 62, RIGHT = 18, TOP = 20, BOTTOM = 24;
export function annotationLabel(annotation: Annotation, format: DisplayFormat) {
  const value = annotation.value;
  if (value === null) return annotation.text;
  if (format === "ascii") {
    const controls: Record<number, string> = { 0: "NUL", 9: "TAB", 10: "LF", 13: "CR", 27: "ESC", 32: "SP", 127: "DEL" };
    return controls[value] ? `<${controls[value]}>` : value >= 33 && value <= 126 ? String.fromCharCode(value) : `\\x${value.toString(16).padStart(2, "0").toUpperCase()}`;
  }
  if (format === "decimal") return String(value);
  if (format === "binary") return value.toString(2).padStart(8, "0");
  return `0x${value.toString(16).padStart(2, "0").toUpperCase()}`;
}
export function ScopeTrack({ id, envelope, sampleCount, rate, time, voltage, mode, visible, annotations, format, threshold, onTime, onVoltage }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; time: TimeView; voltage: VoltageView } | null>(null);
  const [size, setSize] = useState({ width: 800, height: 300 });
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);
  const [box, setBox] = useState<{ x: number; y: number; endX: number; endY: number } | null>(null);
  const plotWidth = Math.max(1, size.width - LEFT - RIGHT), plotHeight = Math.max(1, size.height - TOP - BOTTOM);
  const duration = Math.max(1 / rate, sampleCount / rate);
  const state = useRef({ time, voltage, rate, duration, plotWidth, plotHeight, onTime, onVoltage });
  state.current = { time, voltage, rate, duration, plotWidth, plotHeight, onTime, onVoltage };
  useEffect(() => {
    if (!host.current) return;
    const observer = new ResizeObserver(([entry]) => setSize({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(host.current); return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const s = state.current;
      const rect = el.getBoundingClientRect();
      const x = Math.max(0, Math.min(1, (event.clientX - rect.left - LEFT) / s.plotWidth));
      const y = Math.max(0, Math.min(1, (event.clientY - rect.top - TOP) / s.plotHeight));
      const factor = Math.exp(Math.max(-1, Math.min(1, event.deltaY * 0.002)));
      if (event.shiftKey) {
        const division = Math.max(0.00001, Math.min(100, s.voltage.division * factor));
        const anchor = s.voltage.center + (0.5 - y) * s.voltage.division * 8;
        s.onVoltage({ division, center: anchor - (0.5 - y) * division * 8 });
      } else {
        const span = s.time.span * factor;
        s.onTime(limitTime({ start: s.time.start + x * (s.time.span - span), span }, s.duration, s.rate));
      }
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  }, []);
  useEffect(() => {
    const el = canvas.current, ctx = el?.getContext("2d");
    if (!el || !ctx) return;
    const frame = requestAnimationFrame(() => {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.round(size.width * dpr), h = Math.round(size.height * dpr);
    if (el.width !== w || el.height !== h) { el.width = w; el.height = h; }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.width, size.height);
    ctx.font = "10px Consolas, monospace";
    for (let row = 0; row <= 8; row++) {
      const y = TOP + row / 8 * plotHeight;
      ctx.beginPath(); ctx.moveTo(LEFT, y); ctx.lineTo(size.width - RIGHT, y);
      ctx.strokeStyle = row === 4 ? "#d4d4d4" : "#ededed"; ctx.lineWidth = 1; ctx.stroke();
      if (row % 2 === 0) { ctx.fillStyle = "#737373"; ctx.textAlign = "right"; ctx.fillText(`${(voltage.center + (4 - row) * voltage.division).toFixed(voltage.division < 0.01 ? 4 : 2)} V`, LEFT - 10, y + 3); }
    }
    for (let col = 0; col <= 10; col++) {
      const x = LEFT + col / 10 * plotWidth;
      ctx.beginPath(); ctx.moveTo(x, TOP); ctx.lineTo(x, size.height - BOTTOM); ctx.strokeStyle = "#ededed"; ctx.stroke();
      if (col % 2 === 0) { ctx.fillStyle = "#737373"; ctx.textAlign = col === 0 ? "left" : col === 10 ? "right" : "center"; ctx.fillText(timeLabel(time.start + col / 10 * time.span), x, size.height - 7); }
    }
    const thresholdY = TOP + (0.5 - (threshold - voltage.center) / (voltage.division * 8)) * plotHeight;
    if (Number.isFinite(thresholdY) && thresholdY >= TOP && thresholdY <= size.height - BOTTOM) {
      ctx.setLineDash([5, 5]); ctx.strokeStyle = "#a3a3a3"; ctx.beginPath(); ctx.moveTo(LEFT, thresholdY); ctx.lineTo(size.width - RIGHT, thresholdY); ctx.stroke(); ctx.setLineDash([]);
    }
    if (visible && envelope.min.length) {
      ctx.save(); ctx.beginPath(); ctx.rect(LEFT, TOP, plotWidth, plotHeight); ctx.clip();
      const yFor = (v: number) => TOP + (0.5 - (v - voltage.center) / (voltage.division * 8)) * plotHeight;
      ctx.beginPath(); let begun = false;
      for (let i = 0; i < envelope.min.length; i++) {
        const px = LEFT + (envelope.start_s + i * envelope.step_s - time.start) / time.span * plotWidth;
        const lo = yFor(envelope.min[i]), hi = yFor(envelope.max[i]);
        if (!begun) { ctx.moveTo(px, lo); begun = true; } else ctx.lineTo(px, lo);
        if (lo !== hi) { ctx.lineTo(px, hi); ctx.moveTo(px, (lo + hi) / 2); }
      }
      ctx.strokeStyle = COLORS[id]; ctx.lineWidth = 1.1; ctx.stroke(); ctx.restore();
    }
    });
    return () => cancelAnimationFrame(frame);
  }, [id, envelope, time, voltage, visible, size, plotWidth, plotHeight, threshold]);

  function point(event: React.PointerEvent) {
    const rect = host.current!.getBoundingClientRect();
    return { x: Math.max(LEFT, Math.min(size.width - RIGHT, event.clientX - rect.left)), y: Math.max(TOP, Math.min(size.height - BOTTOM, event.clientY - rect.top)) };
  }
  function finish(event: React.PointerEvent) {
    const start = drag.current;
    if (start && mode === "window") {
      const end = point(event);
      if (Math.abs(end.x - start.x) > 6) onTime(limitTime({ start: start.time.start + (Math.min(start.x, end.x) - LEFT) / plotWidth * start.time.span, span: Math.abs(end.x - start.x) / plotWidth * start.time.span }, duration, rate));
      if (Math.abs(end.y - start.y) > 6) onVoltage({ center: start.voltage.center + (0.5 - ((start.y + end.y) / 2 - TOP) / plotHeight) * start.voltage.division * 8, division: Math.max(0.00001, Math.abs(end.y - start.y) / plotHeight * start.voltage.division) });
    }
    drag.current = null; setBox(null);
    if (host.current?.hasPointerCapture(event.pointerId)) host.current.releasePointerCapture(event.pointerId);
  }
  const inView = useMemo(() => {
    const data = annotations.some(a => a.value !== null) ? annotations.filter(a => a.value !== null || /warning|error/i.test(a.row)) : annotations;
    return data.filter(a => a.end_s >= time.start && a.start_s <= time.start + time.span);
  }, [annotations, time]);
  const displayed = inView.slice(0, 200);
  return <div ref={host} className={`scope-track ${mode}`} role="img" aria-label={`A${id + 1} voltage versus time waveform`}
    onDoubleClick={() => { onTime({ start: 0, span: duration }); onVoltage(fitVoltage([...envelope.min, ...envelope.max])); }}
    onPointerDown={event => { if (event.button !== 0) return; const p = point(event); event.currentTarget.setPointerCapture(event.pointerId); drag.current = { ...p, time, voltage }; if (mode === "window") setBox({ ...p, endX: p.x, endY: p.y }); }}
    onPointerMove={event => { const p = point(event); setCursor(p); const start = drag.current; if (!start) return; if (mode === "window") setBox({ x: start.x, y: start.y, endX: p.x, endY: p.y }); else { onTime(limitTime({ start: start.time.start - (p.x - start.x) / plotWidth * start.time.span, span: start.time.span }, duration, rate)); onVoltage({ ...start.voltage, center: start.voltage.center + (p.y - start.y) / plotHeight * start.voltage.division * 8 }); } }}
    onPointerUp={finish} onPointerCancel={() => { drag.current = null; setBox(null); }} onPointerLeave={() => { if (!drag.current) setCursor(null); }}>
    <canvas ref={canvas} />
    {displayed.length > 0 && <div className="decode-overlays">{displayed.map((annotation, index) => {
      const left = Math.max(0, (annotation.start_s - time.start) / time.span * plotWidth);
      const right = Math.min(plotWidth, (annotation.end_s - time.start) / time.span * plotWidth);
      return <Button variant="outline" size="sm" key={index} className={`decode-bubble ${/warning|error/i.test(annotation.row) ? "warning" : ""}`} style={{ left, width: Math.max(2, right - left), borderColor: COLORS[id] + "90" }} title={`${annotation.row}: ${annotationLabel(annotation, format)} · ${timeLabel(annotation.start_s)}–${timeLabel(annotation.end_s)}`} onPointerDown={e => e.stopPropagation()} onDoubleClick={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); const span = Math.max(8 / rate, (annotation.end_s - annotation.start_s) * 3); onTime(limitTime({ start: annotation.start_s - span / 3, span }, duration, rate)); }}>{right - left > 12 ? annotationLabel(annotation, format) : ""}</Button>;
    })}</div>}
    {annotations.length > 0 && <div className="decode-count">{inView.length.toLocaleString()} decoded fields in view{inView.length > 200 ? " · zoom in for labels" : ""}</div>}

    {box && <div className="zoom-box" style={{ left: Math.min(box.x, box.endX), top: Math.min(box.y, box.endY), width: Math.abs(box.endX - box.x), height: Math.abs(box.endY - box.y) }} />}
    {cursor && !box && <><div className="scope-crosshair vertical" style={{ left: cursor.x, top: TOP, height: plotHeight }} /><div className="scope-crosshair horizontal" style={{ top: cursor.y, left: LEFT, width: plotWidth }} /></>}
    {cursor && <div className="cursor-readout">{timeLabel(time.start + (cursor.x - LEFT) / plotWidth * time.span)}<span>{(voltage.center + (0.5 - (cursor.y - TOP) / plotHeight) * voltage.division * 8).toFixed(4)} V</span></div>}
    {!sampleCount && <div className="trace-empty">A{id + 1}<span>Press Start</span></div>}
  </div>;
}
