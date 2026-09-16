import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
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
  id: number; envelope: Envelope; sampleCount: number; rate: number; duration: number; time: TimeView; voltage: VoltageView;
  mode: "pan" | "window"; visible: boolean;
  annotations: Annotation[]; format: DisplayFormat; threshold: number;
  onTime: (view: TimeView) => void; onVoltage: (view: VoltageView) => void;
};
type PlotBox = { left: number; top: number; width: number; height: number };
const LEFT = 62, RIGHT = 18, TOP = 48, BOTTOM = 32;
const ANNOTATION_HEIGHT = 23;
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));

function timeSplits(_self: uPlot, _axisIdx: number, min: number, max: number) {
  return Array.from({ length: 11 }, (_, index) => min + (max - min) * index / 10);
}
function timeValues(_self: uPlot, splits: number[]) {
  return splits.map((value, index) => index % 2 === 0 ? timeLabel(value) : null);
}
function voltageSplits(_self: uPlot, _axisIdx: number, min: number, max: number) {
  return Array.from({ length: 9 }, (_, index) => min + (max - min) * index / 8);
}
function voltageValues(_self: uPlot, splits: number[]) {
  return splits.map((value, index) => index % 2 === 0 ? `${value.toFixed(Math.abs(value) < 0.01 ? 4 : 2)} V` : null);
}

export function annotationLabel(annotation: Annotation, format: DisplayFormat) {
  // sigrok CAN stores payload bytes in field text, unlike UART's numeric RX
  // annotations. Format these in-place so already captured data also responds.
  const canByte = /^Data byte (\d+):\s*0x([0-9a-f]{1,2})$/i.exec(annotation.text);
  const value = annotation.value ?? (canByte ? Number.parseInt(canByte[2], 16) : null);
  if (value === null) return annotation.text;
  const prefix = canByte ? `D${canByte[1]}: ` : "";
  if (format === "ascii") {
    const controls: Record<number, string> = { 0: "NUL", 9: "TAB", 10: "LF", 13: "CR", 27: "ESC", 32: "SP", 127: "DEL" };
    return prefix + (controls[value] ? `<${controls[value]}>` : value >= 33 && value <= 126 ? String.fromCharCode(value) : `\\x${value.toString(16).padStart(2, "0").toUpperCase()}`);
  }
  if (format === "decimal") return prefix + String(value);
  if (format === "binary") return prefix + value.toString(2).padStart(8, "0");
  return `${prefix}0x${value.toString(16).padStart(2, "0").toUpperCase()}`;
}

export function ScopeTrack({ id, envelope, sampleCount, rate, duration: viewDuration, time, voltage, mode, visible, annotations, format, threshold, onTime, onVoltage }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const plotHost = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);
  const drag = useRef<{ x: number; y: number; time: TimeView; voltage: VoltageView } | null>(null);
  const [size, setSize] = useState({ width: 800, height: 300 });
  const [plotBox, setPlotBox] = useState<PlotBox>({ left: LEFT, top: TOP, width: 800 - LEFT - RIGHT, height: 300 - TOP - BOTTOM });
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);
  const [box, setBox] = useState<{ x: number; y: number; endX: number; endY: number } | null>(null);
  const [hoveredAnnotation, setHoveredAnnotation] = useState<Annotation | null>(null);
  const duration = Math.max(1 / rate, viewDuration);
  const state = useRef({ time, voltage, rate, duration, plotBox, onTime, onVoltage });
  state.current = { time, voltage, rate, duration, plotBox, onTime, onVoltage };

  const data = useMemo<uPlot.AlignedData>(() => {
    const count = Math.min(envelope.min.length, envelope.max.length);
    const x = new Float64Array(count);
    const line = new Float64Array(count);
    for (let index = 0; index < count; index++) {
      x[index] = envelope.start_s + index * envelope.step_s;
      line[index] = (envelope.min[index] + envelope.max[index]) / 2;
    }
    return [x, line];
  }, [envelope]);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setSize({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (!plotHost.current || plot.current) return;
    const updatePlotBox = (chart: uPlot) => {
      // bbox is in canvas pixels; pointer events and overlays use CSS pixels.
      const { left, top, width, height } = chart.over.getBoundingClientRect();
      const hostRect = host.current!.getBoundingClientRect();
      const next = { left: left - hostRect.left, top: top - hostRect.top, width: Math.max(1, width), height: Math.max(1, height) };
      setPlotBox(previous => Object.keys(next).every(key => previous[key as keyof PlotBox] === next[key as keyof PlotBox]) ? previous : next);
    };
    const chart = new uPlot({
      width: size.width,
      height: size.height,
      padding: [TOP, RIGHT, 0, 0],
      scales: {
        x: { time: false, auto: false, min: 0, max: 1 },
        y: { auto: false, min: -4, max: 4 },
      },
      axes: [
        { scale: "x", side: 2, size: BOTTOM, gap: 4, font: "10px Consolas, monospace", stroke: "#737373", splits: timeSplits, values: timeValues, grid: { stroke: "#ededed", width: 1 }, ticks: { stroke: "#d4d4d4", width: 1, size: 4 } },
        { scale: "y", side: 3, size: LEFT, gap: 10, font: "10px Consolas, monospace", stroke: "#737373", splits: voltageSplits, values: voltageValues, grid: { stroke: "#ededed", width: 1 }, ticks: { stroke: "#d4d4d4", width: 1 } },
      ],
      series: [
        {},
        { label: "Voltage", stroke: COLORS[id], width: 1.1, show: visible, points: { show: false } },
      ],
      hooks: { ready: [updatePlotBox], setSize: [updatePlotBox] },
      cursor: { show: false, drag: { setScale: false, x: false, y: false } },
      legend: { show: false },
    }, [[], [], []], plotHost.current);
    plot.current = chart;
    return () => { chart.destroy(); plot.current = null; };
  }, [id]);

  useEffect(() => {
    const chart = plot.current;
    if (!chart) return;
    chart.setSize({ width: Math.max(1, size.width), height: Math.max(1, size.height) });
  }, [size]);

  useEffect(() => {
    const chart = plot.current;
    if (!chart) return;
    const frame = requestAnimationFrame(() => chart.batch(() => {
      if (chart.data !== data) chart.setData(data, false);
      // setSeries invalidates its scale, even when visibility is unchanged.
      // Only toggle when necessary and always apply explicit ranges last.
      if (chart.series[1].show !== visible) {
        chart.setSeries(1, { show: visible }, false);
      }
      chart.setScale("x", { min: time.start, max: time.start + time.span });
      chart.setScale("y", { min: voltage.center - voltage.division * 4, max: voltage.center + voltage.division * 4 });
    }));
    return () => cancelAnimationFrame(frame);
  }, [data, time, voltage, visible]);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const s = state.current;
      const rect = el.getBoundingClientRect();
      const x = clamp((event.clientX - rect.left - s.plotBox.left) / s.plotBox.width, 0, 1);
      const y = clamp((event.clientY - rect.top - s.plotBox.top) / s.plotBox.height, 0, 1);
      const factor = Math.exp(clamp(event.deltaY * 0.002, -1, 1));
      if (event.shiftKey) {
        const division = clamp(s.voltage.division * factor, 0.00001, 100);
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

  function point(event: React.PointerEvent) {
    const rect = host.current!.getBoundingClientRect();
    return {
      x: clamp(event.clientX - rect.left, plotBox.left, plotBox.left + plotBox.width),
      y: clamp(event.clientY - rect.top, plotBox.top, plotBox.top + plotBox.height),
    };
  }
  function finish(event: React.PointerEvent) {
    const start = drag.current;
    if (start && mode === "window") {
      const end = point(event);
      if (Math.abs(end.x - start.x) > 6) onTime(limitTime({ start: start.time.start + (Math.min(start.x, end.x) - plotBox.left) / plotBox.width * start.time.span, span: Math.abs(end.x - start.x) / plotBox.width * start.time.span }, duration, rate));
      if (Math.abs(end.y - start.y) > 6) onVoltage({ center: start.voltage.center + (0.5 - ((start.y + end.y) / 2 - plotBox.top) / plotBox.height) * start.voltage.division * 8, division: Math.max(0.00001, Math.abs(end.y - start.y) / plotBox.height * start.voltage.division) });
    }
    drag.current = null; setBox(null);
    if (host.current?.hasPointerCapture(event.pointerId)) host.current.releasePointerCapture(event.pointerId);
  }

  const inView = useMemo(() => {
    const filtered = annotations.some(a => a.value !== null) ? annotations.filter(a => a.value !== null || /warning|error/i.test(a.row)) : annotations;
    return filtered.filter(a => a.end_s >= time.start && a.start_s <= time.start + time.span);
  }, [annotations, time]);
  const displayed = useMemo(() => {
    const labels: { annotation: Annotation; label: string; left: number; width: number }[] = [];
    // 11px monospace text needs at most ~7px per character plus button padding.
    // Skip narrow fields completely: empty/slivered boxes obscure the trace.
    for (const annotation of inView) {
      const left = Math.max(0, (annotation.start_s - time.start) / time.span * plotBox.width);
      const right = Math.min(plotBox.width, (annotation.end_s - time.start) / time.span * plotBox.width);
      const label = annotationLabel(annotation, format);
      if (right - left < label.length * 7 + 12) continue;
      labels.push({ annotation, label, left, width: right - left });
      if (labels.length === 200) break;
    }
    return labels;
  }, [inView, time, plotBox.width, format]);
  const highlighted = useMemo(() => {
    if (!hoveredAnnotation || hoveredAnnotation.end_s < time.start || hoveredAnnotation.start_s > time.start + time.span) return null;
    const left = Math.max(0, (hoveredAnnotation.start_s - time.start) / time.span * plotBox.width);
    const right = Math.min(plotBox.width, (hoveredAnnotation.end_s - time.start) / time.span * plotBox.width);
    return { left, width: Math.max(1, right - left) };
  }, [hoveredAnnotation, time, plotBox.width]);
  const thresholdY = plotBox.top + (0.5 - (threshold - voltage.center) / (voltage.division * 8)) * plotBox.height;
  const thresholdVisible = Number.isFinite(thresholdY) && thresholdY >= plotBox.top && thresholdY <= plotBox.top + plotBox.height;

  return <div ref={host} className={`scope-track ${mode}`} role="img" aria-label={`A${id + 1} voltage versus time waveform`}
    onDoubleClick={() => { onTime({ start: 0, span: duration }); onVoltage(fitVoltage([...envelope.min, ...envelope.max])); }}
    onPointerDown={event => { if (event.button !== 0) return; const p = point(event); event.currentTarget.setPointerCapture(event.pointerId); drag.current = { ...p, time, voltage }; if (mode === "window") setBox({ ...p, endX: p.x, endY: p.y }); }}
    onPointerMove={event => { const p = point(event); setCursor(p); const start = drag.current; if (!start) return; if (mode === "window") setBox({ x: start.x, y: start.y, endX: p.x, endY: p.y }); else onTime(limitTime({ start: start.time.start - (p.x - start.x) / plotBox.width * start.time.span, span: start.time.span }, duration, rate)); }}
    onPointerUp={finish} onPointerCancel={() => { drag.current = null; setBox(null); }} onPointerLeave={() => { if (!drag.current) setCursor(null); }}>
    <div ref={plotHost} className="scope-plot" />
    {thresholdVisible && <div className="logic-threshold-line" style={{ left: plotBox.left, top: thresholdY, width: plotBox.width }} />}
    {highlighted && <div className="decode-highlight" style={{ left: plotBox.left + highlighted.left, top: plotBox.top, width: highlighted.width, height: plotBox.height, borderColor: COLORS[id] + "b0", backgroundColor: COLORS[id] + "18" }} />}
    {displayed.length > 0 && <div className="decode-overlays" style={{ left: plotBox.left, top: plotBox.top - ANNOTATION_HEIGHT - 1, width: plotBox.width }}>{displayed.map(({ annotation, label, left, width }, index) => {
      return <Button variant="outline" size="sm" key={index} className={`decode-bubble ${/warning|error/i.test(annotation.row) ? "warning" : ""}`} style={{ left, width, borderColor: COLORS[id] + "90" }} title={`${annotation.row}: ${label} · ${timeLabel(annotation.start_s)}–${timeLabel(annotation.end_s)}`} onPointerDown={e => e.stopPropagation()} onDoubleClick={e => e.stopPropagation()} onMouseEnter={() => setHoveredAnnotation(annotation)} onMouseLeave={() => setHoveredAnnotation(null)} onFocus={() => setHoveredAnnotation(annotation)} onBlur={() => setHoveredAnnotation(null)} onClick={e => { e.stopPropagation(); const span = Math.max(8 / rate, (annotation.end_s - annotation.start_s) * 3); onTime(limitTime({ start: annotation.start_s - span / 3, span }, duration, rate)); }}>{label}</Button>;
    })}</div>}
    {annotations.length > 0 && <div className="decode-count">{inView.length.toLocaleString()} decoded fields in view{displayed.length < inView.length ? " · zoom in for more labels" : ""}</div>}
    {box && <div className="zoom-box" style={{ left: Math.min(box.x, box.endX), top: Math.min(box.y, box.endY), width: Math.abs(box.endX - box.x), height: Math.abs(box.endY - box.y) }} />}
    {cursor && !box && <><div className="scope-crosshair vertical" style={{ left: cursor.x, top: plotBox.top, height: plotBox.height }} /><div className="scope-crosshair horizontal" style={{ top: cursor.y, left: plotBox.left, width: plotBox.width }} /></>}
    {cursor && <div className="cursor-readout">{timeLabel(time.start + (cursor.x - plotBox.left) / plotBox.width * time.span)}<span>{(voltage.center + (0.5 - (cursor.y - plotBox.top) / plotBox.height) * voltage.division * 8).toFixed(4)} V</span></div>}
    {!sampleCount && <div className="trace-empty">A{id + 1}<span>Press Start</span></div>}
  </div>;
}
