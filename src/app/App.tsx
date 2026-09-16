import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, GripVertical, X, RefreshCw, LoaderCircle, Play, Square } from "lucide-react";
import { ScopeTrack, COLORS, limitTime, timeLabel, type TimeView, type VoltageView } from "./ScopeTrack";
export type Envelope = { start_s: number; step_s: number; min: number[]; max: number[] };
type StreamView = { running: boolean; directory: string; sample_rate_hz: number; adc_bits: number; total_samples: number; samples_lost: number; samples_corrupt: number; sample_count: number; origin_s: number; duration_s: number; sequence: number; error: string | null; envelopes: Envelope[]; stats: { min_v: number; max_v: number; mean_v: number }[] };
function fitStats(stats?: { min_v: number; max_v: number }): VoltageView { return stats ? { center: (stats.min_v + stats.max_v) / 2, division: Math.max(0.001, (stats.max_v - stats.min_v) / 6) } : { center: 0, division: 1 }; }
const EMPTY_ENVELOPE: Envelope = { start_s: 0, step_s: 0, min: [], max: [] };
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubTrigger, DropdownMenuSubContent } from "@/components/ui/dropdown-menu";
import { PresetMenu, BIT_RATES, THRESHOLDS, VOLTAGE_SCALES, SAMPLE_RATES, WINDOW_LENGTHS } from "./PresetMenu";
import { Accordion, AccordionItem, AccordionTrigger, AccordionContent } from "@/components/ui/accordion";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Sidebar, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent, SidebarGroupLabel, SidebarHeader, SidebarInset, SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar";
import "../App.css";

export type Annotation = { start_s: number; end_s: number; row: string; text: string; value: number | null };
export type DisplayFormat = "hex" | "ascii" | "decimal" | "binary";
type DecoderConfig = { protocol: string; bitrate: string; threshold: string; expression: string };
type DecodeResult = { annotations: Annotation[]; transitions: number[]; directory: string; origin_s: number };
type Device = { index: number; name: string; serial: string; in_use: boolean };
const IMU_FIRMWARE_PROTOCOL = "imu_firmware";
const IMU_CAN_BITRATE = "1000000";
const DEFAULT_DECODER: DecoderConfig = { protocol: "uart", bitrate: "115200", threshold: "1.65", expression: "" };
function decoderExpression(config: DecoderConfig, channel: number) {
  if (config.expression.trim()) return config.expression.trim();
  const rate = config.bitrate;
  const expressions: Record<string, string> = {
    uart: `uart:rx=${channel}:baudrate=${rate}:format=hex`,
    can: `can:can_rx=${channel}:nominal_bitrate=${rate}`,
    [IMU_FIRMWARE_PROTOCOL]: `${IMU_FIRMWARE_PROTOCOL}:can_rx=${channel}:nominal_bitrate=${rate}`,
    i2c: "i2c:scl=0:sda=1", spi: "spi:clk=0:mosi=1",
    guess_bitrate: `guess_bitrate:data=${channel}`,
    usb_signalling: "usb_signalling:dp=0:dm=1,usb_packet,usb_request",
    midi: `uart:rx=${channel}:baudrate=31250,midi`,
    modbus: `uart:rx=${channel}:baudrate=${rate},modbus`,
    lin: `uart:rx=${channel}:baudrate=${rate},lin`,
    dmx512: `uart:rx=${channel}:baudrate=250000:stop_bits=2,dmx512`,
    ps2: "ps2:clk=0:data=1", swd: "swd:swclk=0:swdio=1", mdio: "mdio:mdc=0:mdio=1",
    onewire_link: `onewire_link:owr=${channel}`, rgb_led_ws281x: `rgb_led_ws281x:din=${channel}`,
  };
  return expressions[config.protocol] ?? config.protocol;
}
const names: Record<string, string> = { uart: "UART / Serial", can: "CAN / CAN FD", [IMU_FIRMWARE_PROTOCOL]: "IMU firmware", i2c: "I²C", spi: "SPI", guess_bitrate: "Estimate bit rate", usb_signalling: "USB" };

function ScopeSelect({ id, label, value, onValueChange, options, className = "" }: {
  id?: string; label: string; value: string; onValueChange: (value: string) => void;
  options: { value: string; label: string }[]; className?: string;
}) {
  return <Select value={value} onValueChange={onValueChange}>
    <SelectTrigger id={id} aria-label={label} className={`h-8 w-full text-xs ${className}`}><SelectValue placeholder={label} /></SelectTrigger>
    <SelectContent position="popper" className="max-h-80">{options.map(option => <SelectItem key={option.value} value={option.value} className="text-xs">{option.label}</SelectItem>)}</SelectContent>
  </Select>;
}

export default function App() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [device, setDevice] = useState(0);
  const [capture, setCapture] = useState<StreamView | null>(null);
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const operation = useRef(false);
  const firstFrame = useRef(false);
  const justStopped = useRef(false);
  const lastDisplay = useRef<StreamView | null>(null);
  const viewRef = useRef<TimeView>({ start: 0, span: 0.1 });
  const [message, setMessage] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sampleRate, setSampleRate] = useState("1");
  const [durationMs, setDurationMs] = useState("100");
  const [mode, setMode] = useState<"pan" | "window">("window");
  const [format, setFormat] = useState<DisplayFormat>("hex");
  const [time, setTime] = useState<TimeView>({ start: 0, span: 0.1 });
  const [voltage, setVoltage] = useState<VoltageView[]>([{ center: 0, division: 1 }, { center: 0, division: 1 }]);
  const [order, setOrder] = useState([0, 1]);
  const [configs, setConfigs] = useState<DecoderConfig[]>([{ ...DEFAULT_DECODER }, { ...DEFAULT_DECODER }]);
  const [decoders, setDecoders] = useState<{ id: string; name: string }[]>([]);
  const [results, setResults] = useState<(DecodeResult | null)[]>([null, null]);
  const [errors, setErrors] = useState(["", ""]);
  const [decoding, setDecoding] = useState(false);
  const [advanced, setAdvanced] = useState<number | null>(null);
  const [details, setDetails] = useState("");
  const [hysteresis, setHysteresis] = useState("0.1");
  const [differential, setDifferential] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const dragChannel = useRef<number | null>(null);
  const rate = capture?.sample_rate_hz ?? 1e6;
  viewRef.current = time;
  const duration = capture?.duration_s || Number(durationMs) / 1000;
  async function discover() {
    try {
      const found = await invoke<Device[]>("discover_ad3"); setDevices(found);
      if (found.length) setDevice(found[0].index); else setMessage("Connect an Analog Discovery 3, then refresh devices.");
    } catch (e) { setMessage(String(e)); }
  }
  useEffect(() => { void discover(); invoke<{ id: string; name: string }[]>("available_decoders").then(setDecoders).catch(e => setMessage(String(e))); }, []);
  useEffect(() => {
    if (advanced === null || configs[advanced].protocol === "off") return;
    let active = true; setDetails("");
    invoke<string>("decoder_details", { id: configs[advanced].protocol }).then(value => { if (active) setDetails(value); }).catch(e => { if (active) setDetails(String(e)); });
    return () => { active = false; };
  }, [advanced, configs]);
  function updateConfig(channel: number, change: Partial<DecoderConfig>) { setConfigs(current => current.map((config, index) => index === channel ? { ...config, ...change } : config)); }
  function fit() { setTime({ start: 0, span: duration }); if (capture) setVoltage(capture.stats.map(fitStats)); }
  async function toggleStream() {
    if (operation.current) return;
    operation.current = true; setBusy(true); setMessage("");
    try {
      if (running) {
        await invoke("stop_stream"); justStopped.current = true; setRunning(false); setRefresh(r => r + 1);
      } else {
        const hz = Number(sampleRate) * 1e6, windowMs = Number(durationMs);
        if (!Number.isFinite(hz) || hz <= 0 || !Number.isFinite(windowMs) || windowMs <= 0) throw new Error("Enter a positive sample rate and display window.");
        firstFrame.current = true; justStopped.current = false; lastDisplay.current = null; setCapture(null); setResults([null, null]);
        await invoke("start_stream", { index: device, sampleRateHz: hz, windowMs });
        setRunning(true);
      }
    } catch (e) { setMessage(String(e)); } finally { operation.current = false; setBusy(false); }
  }
  useEffect(() => {
    if (!running && !capture) return;
    let active = true; let timer = 0;
    const poll = async () => {
      try {
        const view = await invoke<StreamView>("stream_snapshot", { startS: firstFrame.current ? null : viewRef.current.start, spanS: firstFrame.current ? null : viewRef.current.span, pixels: Math.min(2048, Math.ceil(window.innerWidth)) });
        if (!active) return;
        const retained = view.sample_count === 0 && (running || justStopped.current) ? lastDisplay.current : null;
        const displayView = retained
          ? { ...view, sample_count: retained.sample_count, origin_s: retained.origin_s, duration_s: retained.duration_s, envelopes: retained.envelopes, stats: retained.stats }
          : view;
        if (view.sample_count > 0) lastDisplay.current = view;
        setCapture(displayView);
        if (displayView.error) setMessage(displayView.error);
        if (firstFrame.current && displayView.sample_count > 0) {
          setTime({ start: 0, span: Number(durationMs) / 1000 }); setVoltage(displayView.stats.map(fitStats)); firstFrame.current = false;
        }
        if (!view.running && (running || justStopped.current)) {
          justStopped.current = false;
          setRunning(false);
          const requestedSpan = Number(durationMs) / 1000;
          const finalSpan = displayView.duration_s > 0 ? Math.min(requestedSpan, displayView.duration_s) : requestedSpan;
          const minimumSpan = 1 / view.sample_rate_hz;
          setTime(current => current.start >= 0 && current.start + current.span <= finalSpan + minimumSpan
            ? current
            : { start: 0, span: Math.max(minimumSpan, finalSpan) });
          setVoltage(view.stats.map(fitStats));
        }
      } catch (e) { if (active) { setMessage(String(e)); setRunning(false); } }
      if (active && running) timer = window.setTimeout(poll, 100);
    };
    // Throttle requests during pan/zoom. Each reply contains at most one min/max pair per pixel.
    timer = window.setTimeout(poll, 40);
    return () => { active = false; window.clearTimeout(timer); };
  }, [running, time, refresh, durationMs]);
  useEffect(() => () => { void invoke("stop_stream").catch(() => {}); }, []);
  useEffect(() => {
    setResults([null, null]); setErrors(["", ""]); setDecoding(false);
    if (!capture && !running) return;
    let active = true; let timer = 0;
    const decode = async () => {
      setDecoding(true);
      try {
        const parameters = configs.map(config => {
          if (config.protocol === "off") return null;
          if (!config.bitrate.trim() || !Number.isFinite(Number(config.bitrate)) || Number(config.bitrate) <= 0) throw new Error("Enter a positive bit rate.");
          if (configs.some(c => !c.threshold.trim() || !Number.isFinite(Number(c.threshold))) || !hysteresis.trim() || !Number.isFinite(Number(hysteresis))) throw new Error("Enter numeric thresholds.");
          return { expression: decoderExpression(config, configs.indexOf(config)), thresholds: configs.map(c => Number(c.threshold)), hysteresis_v: Number(hysteresis), differential };
        });
        const decoded = await invoke<{ origin_s: number; results: ({ Ok: DecodeResult } | { Err: string })[] }>("decode_stream", { configs: parameters });
        if (active) {
          setResults(decoded.results.map(result => "Ok" in result ? { ...result.Ok, origin_s: decoded.origin_s } : null));
          setErrors(decoded.results.map(result => "Err" in result && result.Err !== "Decoder off" ? result.Err : ""));
        }
      } catch (e) { if (active && !String(e).includes("Waiting for clean")) setErrors([String(e), String(e)]); }
      if (active) { setDecoding(false); if (running) timer = window.setTimeout(decode, 500); }
    };
    timer = window.setTimeout(decode, running ? 800 : 200);
    return () => { active = false; window.clearTimeout(timer); };
  }, [running, configs, hysteresis, differential, refresh]);
  const protocolList = [{ id: IMU_FIRMWARE_PROTOCOL, name: "IMU firmware" }, ...decoders].sort((a, b) => {
    const common = [IMU_FIRMWARE_PROTOCOL, "uart", "can", "i2c", "spi", "guess_bitrate", "usb_signalling"];
    const ai = common.indexOf(a.id), bi = common.indexOf(b.id);
    return (ai < 0 ? 100 : ai) - (bi < 0 ? 100 : bi) || a.name.localeCompare(b.name);
  });
  const protocolOptions = decoders.length
    ? protocolList.map(d => ({ value: d.id, label: names[d.id] ?? d.name }))
    : [{ value: "uart", label: "UART / Serial" }, { value: IMU_FIRMWARE_PROTOCOL, label: "IMU firmware" }];
  return <TooltipProvider delayDuration={300}>
    {/* Layout adapted from the official shadcn/ui dashboard-01 block. */}
    <SidebarProvider className="h-svh min-h-0" style={{ "--sidebar-width": "17rem", "--header-height": "3.5rem" } as React.CSSProperties}>
      <Sidebar variant="inset" collapsible="offcanvas">
        <SidebarHeader className="h-14 flex-row items-center gap-3 px-4">
          <div><h1 className="text-sm font-semibold tracking-tight">Analog Logic</h1><p className="text-[10px] text-muted-foreground">Voltage · time · decode</p></div>
        </SidebarHeader>
        <SidebarContent className="gap-0">
          {order.map(channel => <SidebarGroup key={channel} className="px-4 pb-4" onDragOver={e => { if (dragChannel.current !== null) e.preventDefault(); }} onDrop={e => { e.preventDefault(); if (dragChannel.current !== null && dragChannel.current !== channel) setOrder(current => [...current].reverse()); dragChannel.current = null; }}>
            <SidebarGroupLabel className="mb-2 h-8 cursor-grab gap-2 px-0" draggable onDragStart={e => { dragChannel.current = channel; e.dataTransfer.setData("text/plain", `A${channel + 1}`); }} onDragEnd={() => { dragChannel.current = null; }} title="Drag to reorder channels">
              <GripVertical className="size-3.5" /><span className="size-2 rounded-sm" style={{ background: COLORS[channel] }} /><span className="text-sm font-semibold" style={{ color: COLORS[channel] }}>A{channel + 1}</span><span className="ml-auto text-[10px] font-normal">Analog input</span>
            </SidebarGroupLabel>
            <SidebarGroupContent className="space-y-3">
              <div className="space-y-1.5"><Label htmlFor={`protocol-${channel}`} className="text-xs text-muted-foreground">Protocol</Label>
                <ScopeSelect id={`protocol-${channel}`} label={`A${channel + 1} protocol`} value={configs[channel].protocol} onValueChange={value => updateConfig(channel, { protocol: value, expression: "", ...(value === "can" || value === IMU_FIRMWARE_PROTOCOL ? { bitrate: IMU_CAN_BITRATE } : {}) })} options={[{ value: "off", label: "Off" }, ...protocolOptions]} />
              </div>
              {configs[channel].protocol !== "off" && <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1.5"><Label htmlFor={`bitrate-${channel}`} className="text-[11px] text-muted-foreground">Bit rate · bit/s</Label><PresetMenu id={`bitrate-${channel}`} label={`A${channel + 1} bit rate`} value={configs[channel].bitrate} options={BIT_RATES} min={1} unit="bit/s" onChange={value => updateConfig(channel, { bitrate: value })} /></div>
                <div className="space-y-1.5"><Label htmlFor={`threshold-${channel}`} className="text-[11px] text-muted-foreground">Threshold · V</Label><PresetMenu id={`threshold-${channel}`} label={`A${channel + 1} logic threshold`} value={configs[channel].threshold} options={THRESHOLDS} unit="V" onChange={value => updateConfig(channel, { threshold: value })} /></div>
              </div>}
              <div className="space-y-1.5"><Label htmlFor={`voltage-${channel}`} className="text-[11px] text-muted-foreground">Voltage scale</Label>
                <PresetMenu id={`voltage-${channel}`} label={`A${channel + 1} voltage scale`} value={String(voltage[channel].division)} options={VOLTAGE_SCALES} unit="V/div" min={0.00001} onChange={value => setVoltage(v => v.map((old, i) => i === channel ? { ...old, division: Number(value) } : old))} action={{ label: "Fit voltage to signal", run: () => setVoltage(v => v.map((old, i) => i === channel ? fitStats(capture?.stats[channel]) : old)) }} />
              </div>
              <div className="flex items-center justify-between gap-2"><span className="text-[10px] text-muted-foreground">{decoding ? "Decoding…" : results[channel] ? `${results[channel]!.annotations.filter(a => a.value !== null).length} values · ${results[channel]!.transitions[channel]} edges` : capture ? "No decode" : "No data"}</span>
                <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="sm" className="h-6 px-1 text-[10px]" aria-label={`A${channel + 1} options`}>Options<ChevronDown className="size-3" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end"><DropdownMenuLabel>A{channel + 1}</DropdownMenuLabel><DropdownMenuItem onSelect={() => setVoltage(v => v.map((old, i) => i === channel ? fitStats(capture?.stats[channel]) : old))}>Fit voltage</DropdownMenuItem><DropdownMenuItem disabled={configs[channel].protocol === "off"} onSelect={() => setAdvanced(channel)}>Decoder options…</DropdownMenuItem></DropdownMenuContent></DropdownMenu>
              </div>
              {errors[channel] && <p className="max-h-20 overflow-auto break-words text-[11px] text-destructive" role="alert">{errors[channel]}</p>}
            </SidebarGroupContent>
            {channel === order[0] && <Separator className="mt-5" />}
          </SidebarGroup>)}
        </SidebarContent>
        <SidebarFooter className="gap-3 p-4">
          <Separator /><div className="flex items-center gap-2"><span className={`size-1.5 rounded-full ${devices.length ? "bg-primary" : "bg-muted-foreground"}`} /><span className="text-xs text-muted-foreground">{devices.length ? "AD3 connected" : "No device"}</span><Button variant="ghost" size="icon-xs" className="ml-auto" aria-label="Refresh devices" disabled={running || busy} onClick={discover}><RefreshCw /></Button></div>
          {devices.length > 1 && <ScopeSelect label="AD3 device" value={String(device)} onValueChange={value => setDevice(Number(value))} options={devices.map(d => ({ value: String(d.index), label: `${d.name} · ${d.serial}` }))} />}
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="min-w-0 overflow-hidden border md:my-2 md:mr-2">
        <header className="flex h-14 shrink-0 items-center gap-3 border-b px-4">
          <SidebarTrigger aria-label="Toggle channel sidebar" /><Separator orientation="vertical" className="h-4!" /><h2 className="text-sm font-medium">Scope</h2>
          <Badge variant="outline" className="hidden text-[10px] lg:inline-flex">A1 + A2</Badge>
          <div className="ml-auto flex items-center gap-3"><span className="hidden text-xs text-muted-foreground sm:inline">{sampleRate} MS/s · {durationMs} ms live window</span>
            <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="sm" className="text-xs" aria-label="Stream settings" disabled={running || busy}>Stream<ChevronDown className="size-3" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end" className="w-56">
              <DropdownMenuLabel>Stream settings</DropdownMenuLabel>
              <DropdownMenuSub><DropdownMenuSubTrigger>Sample rate · {sampleRate} MS/s</DropdownMenuSubTrigger><DropdownMenuSubContent><DropdownMenuRadioGroup value={sampleRate} onValueChange={setSampleRate}>{SAMPLE_RATES.map(option => <DropdownMenuRadioItem key={option.value} value={option.value}>{option.label}</DropdownMenuRadioItem>)}</DropdownMenuRadioGroup></DropdownMenuSubContent></DropdownMenuSub>
              <DropdownMenuSub><DropdownMenuSubTrigger>Live window · {durationMs} ms</DropdownMenuSubTrigger><DropdownMenuSubContent><DropdownMenuRadioGroup value={durationMs} onValueChange={setDurationMs}>{WINDOW_LENGTHS.map(option => <DropdownMenuRadioItem key={option.value} value={option.value}>{option.label}</DropdownMenuRadioItem>)}</DropdownMenuRadioGroup></DropdownMenuSubContent></DropdownMenuSub>
              <DropdownMenuSeparator /><DropdownMenuItem onSelect={() => setSettingsOpen(true)}>Custom stream settings…</DropdownMenuItem>
            </DropdownMenuContent></DropdownMenu>
            <Button size="icon-sm" disabled={busy || (!running && !devices.length)} onClick={toggleStream} aria-label={running ? "Stop stream" : "Start stream"} aria-busy={busy} title={busy ? (running ? "Stopping…" : "Starting…") : running ? "Stop stream" : "Start stream"}>
              {busy ? <LoaderCircle className="animate-spin" aria-hidden="true" /> : running ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
              <span className="sr-only">{busy ? (running ? "Stopping…" : "Starting…") : running ? "Stop" : "Start"}</span>
            </Button>
          </div>
        </header>
        <div className="flex h-12 shrink-0 items-center gap-3 border-b px-4">
          <DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline" size="sm" className="text-xs" aria-label="View controls">{mode === "pan" ? "Pan" : "Window zoom"}<ChevronDown className="size-3" /></Button></DropdownMenuTrigger><DropdownMenuContent align="start"><DropdownMenuLabel>View</DropdownMenuLabel><DropdownMenuRadioGroup value={mode} onValueChange={value => { if (value === "pan" || value === "window") setMode(value); }}><DropdownMenuRadioItem value="pan">Pan</DropdownMenuRadioItem><DropdownMenuRadioItem value="window">Window zoom</DropdownMenuRadioItem></DropdownMenuRadioGroup><DropdownMenuSeparator /><DropdownMenuItem onSelect={fit}>Fit both traces</DropdownMenuItem></DropdownMenuContent></DropdownMenu>
          <div className="ml-auto flex items-center gap-2"><Label htmlFor="decode-format" className="text-xs text-muted-foreground">Decode as</Label><ScopeSelect id="decode-format" label="Decoded display format" className="w-28" value={format} onValueChange={value => setFormat(value as DisplayFormat)} options={[{ value: "hex", label: "Hex" }, { value: "ascii", label: "ASCII" }, { value: "decimal", label: "Decimal" }, { value: "binary", label: "Binary" }]} /></div>
        </div>
        {message && <div className="flex items-center justify-between gap-2 border-b border-destructive/20 bg-destructive/10 px-4 py-2 text-xs" role="status"><span>{message}</span><Button variant="ghost" size="icon-xs" aria-label="Dismiss message" onClick={() => setMessage("")}><X /></Button></div>}
        <main className="flex min-h-0 flex-1 flex-col">
          {order.map(channel => <section key={channel} className="relative flex min-h-0 flex-1 flex-col border-b last:border-b-0" aria-label={`A${channel + 1} scope`}>
            <div className="pointer-events-none absolute left-[66px] top-2 z-10 flex items-center gap-2"><Badge variant="outline" className="h-5 rounded-sm px-1.5 text-[10px]" style={{ color: COLORS[channel], borderColor: COLORS[channel] + "55" }}>A{channel + 1}</Badge><span className="text-[10px] text-muted-foreground">{configs[channel].protocol === "off" ? "Analog" : names[configs[channel].protocol] ?? configs[channel].protocol}</span></div>
            <ScopeTrack id={channel} envelope={capture?.envelopes[channel] ?? EMPTY_ENVELOPE} sampleCount={capture?.sample_count ?? 0} rate={rate} duration={capture?.duration_s ?? Number(durationMs) / 1000} time={time} voltage={voltage[channel]} mode={mode} visible={true} annotations={results[channel]?.annotations.map(a => ({ ...a, start_s: a.start_s + results[channel]!.origin_s - (capture?.origin_s ?? 0), end_s: a.end_s + results[channel]!.origin_s - (capture?.origin_s ?? 0) })) ?? []} format={format} threshold={Number(configs[channel].threshold)} onTime={value => setTime(limitTime(value, duration, rate))} onVoltage={value => setVoltage(v => v.map((old, i) => i === channel ? value : old))} />
          </section>)}
        </main>
        <footer className="flex h-9 shrink-0 items-center gap-3 border-t px-4 text-[10px] text-muted-foreground"><span>{capture ? `${running ? "Live" : "Stopped"} · ${capture.total_samples.toLocaleString()} samples/ch · ${capture.samples_lost} lost · ${capture.samples_corrupt} corrupt` : "Scroll: time zoom · Shift + scroll: voltage · Drag: select zoom window"}</span>{capture && <Badge variant="secondary" className="text-[9px]" title={capture.directory}>Stream logged</Badge>}<span className="ml-auto font-mono">{timeLabel((capture?.origin_s ?? 0) + time.start)} — {timeLabel((capture?.origin_s ?? 0) + time.start + time.span)}</span></footer>
      </SidebarInset>
    </SidebarProvider>
    <Dialog open={settingsOpen} onOpenChange={setSettingsOpen}><DialogContent><DialogHeader><DialogTitle>Stream settings</DialogTitle><DialogDescription>Both analog inputs stream until Stop. The live window stays on screen while the full stream is logged.</DialogDescription></DialogHeader>

      <div className="grid grid-cols-2 gap-4"><div className="space-y-2"><Label htmlFor="sample-rate">Sample rate · MS/s</Label><Input id="sample-rate" type="number" min="0.001" max="125" step="1" value={sampleRate} onChange={e => setSampleRate(e.target.value)} /></div><div className="space-y-2"><Label htmlFor="duration">Live window · ms</Label><Input id="duration" type="number" min="0.001" step="1" value={durationMs} onChange={e => setDurationMs(e.target.value)} /></div></div>
      <DialogFooter><Button onClick={() => setSettingsOpen(false)}>Done</Button></DialogFooter>
    </DialogContent></Dialog>
    <Dialog open={advanced !== null} onOpenChange={open => { if (!open) setAdvanced(null); }}><DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
      <DialogHeader><DialogTitle>A{(advanced ?? 0) + 1} decoder options</DialogTitle><DialogDescription>Channel 0 is A1; channel 1 is A2. Set clock/data roles, custom rates, or decoder stacks here.</DialogDescription></DialogHeader>
      {advanced !== null && <><div className="space-y-2"><Label htmlFor="decoder-expression">Channel mapping and options</Label><Textarea id="decoder-expression" className="font-mono text-xs" value={configs[advanced].expression || decoderExpression(configs[advanced], advanced)} onChange={e => updateConfig(advanced, { expression: e.target.value })} /></div>
        <div className="space-y-2"><Label htmlFor="hysteresis">Hysteresis · V</Label><ScopeSelect id="hysteresis" label="Hysteresis" value={hysteresis} onValueChange={setHysteresis} options={[...new Set(["0", "0.01", "0.02", "0.05", "0.1", "0.2", "0.5", hysteresis])].map(value => ({ value, label: `${value} V` }))} /></div>
        <div className="flex items-center justify-between gap-4 rounded-lg border p-3"><Label htmlFor="differential" className="text-xs leading-relaxed">CAN-L on A1 / CAN-H on A2<br /><span className="font-normal text-muted-foreground">Decode the differential signal on A1; keep A2 as the companion trace</span></Label><Switch id="differential" checked={differential} onCheckedChange={checked => { setDifferential(checked); if (checked) { updateConfig(0, { threshold: "0.9" }); updateConfig(1, { protocol: "off", expression: "" }); } }} /></div>
        <Accordion type="single" collapsible><AccordionItem value="options"><AccordionTrigger className="text-xs">Supported channel roles and options</AccordionTrigger><AccordionContent><pre className="max-h-64 overflow-auto whitespace-pre-wrap text-[10px] leading-relaxed text-muted-foreground">{details}</pre></AccordionContent></AccordionItem></Accordion>
      </>}
      <DialogFooter><Button onClick={() => { setAdvanced(null); setRefresh(r => r + 1); }}>Apply</Button></DialogFooter>
    </DialogContent></Dialog>
  </TooltipProvider>;
}
