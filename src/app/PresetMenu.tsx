import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";

type Option = { value: string; label: string };

/** Shadcn preset dropdown with an explicit custom-value escape hatch. */
export function PresetMenu({ id, label, value, options, onChange, unit = "", min, max, disabled, action }: {
  id: string; label: string; value: string; options: Option[];
  onChange: (value: string) => void; unit?: string; min?: number; max?: number;
  disabled?: boolean; action?: { label: string; run: () => void };
}) {
  const [customOpen, setCustomOpen] = useState(false);
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState("");
  const selected = options.find(option => Number(option.value) === Number(value));
  const currentLabel = selected?.label ?? `${Number(value).toLocaleString(undefined, { maximumSignificantDigits: 5 })}${unit ? ` ${unit}` : ""}`;
  function apply() {
    const number = Number(draft);
    if (!draft.trim() || !Number.isFinite(number) || (min !== undefined && number < min) || (max !== undefined && number > max)) {
      setError(`Enter a finite number${min !== undefined ? ` ≥ ${min}` : ""}${max !== undefined ? ` and ≤ ${max}` : ""}.`);
      return;
    }
    onChange(String(number)); setCustomOpen(false);
  }
  return <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button id={id} variant="outline" size="sm" aria-label={label} disabled={disabled} className="w-full justify-between font-normal text-xs"><span className="truncate">{currentLabel}</span><ChevronDown className="size-3.5 text-muted-foreground" /></Button></DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-52" onCloseAutoFocus={event => { if (customOpen) event.preventDefault(); }}>
        <DropdownMenuLabel className="text-xs">{label}</DropdownMenuLabel>
        {action && <><DropdownMenuItem className="text-xs" onSelect={action.run}>{action.label}</DropdownMenuItem><DropdownMenuSeparator /></>}
        <DropdownMenuRadioGroup value={selected?.value ?? value} onValueChange={onChange} className="max-h-64 overflow-y-auto">
          {!selected && <DropdownMenuRadioItem value={value} className="text-xs">{currentLabel} · custom</DropdownMenuRadioItem>}
          {options.map(option => <DropdownMenuRadioItem key={option.value} value={option.value} className="text-xs">{option.label}</DropdownMenuRadioItem>)}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-xs" onSelect={() => { setDraft(value); setError(""); setCustomOpen(true); }}>Custom value…</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <Dialog open={customOpen} onOpenChange={setCustomOpen}><DialogContent className="sm:max-w-sm">
      <form onSubmit={event => { event.preventDefault(); apply(); }} className="space-y-4">
        <DialogHeader><DialogTitle>{label}</DialogTitle><DialogDescription>Enter a custom value{unit ? ` in ${unit}` : ""}.</DialogDescription></DialogHeader>
        <div className="space-y-2"><Label htmlFor={`${id}-custom`}>Value{unit ? ` · ${unit}` : ""}</Label><Input id={`${id}-custom`} type="number" step="any" min={min} max={max} value={draft} onChange={event => setDraft(event.target.value)} autoFocus /></div>
        {error && <p className="text-xs text-destructive" role="alert">{error}</p>}
        <DialogFooter><Button type="button" variant="outline" onClick={() => setCustomOpen(false)}>Cancel</Button><Button type="submit">Apply</Button></DialogFooter>
      </form>
    </DialogContent></Dialog>
  </>;
}

export const BIT_RATES = [1200, 2400, 4800, 9600, 19200, 31250, 38400, 57600, 115200, 125000, 230400, 250000, 460800, 500000, 921600, 1000000, 2000000, 5000000, 10000000].map(value => ({ value: String(value), label: value.toLocaleString() }));
export const THRESHOLDS = [-1, 0, 0.5, 0.9, 1, 1.2, 1.65, 2.5, 3.3, 5].map(value => ({ value: String(value), label: `${value} V` }));
export const VOLTAGE_SCALES = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10].map(value => ({ value: String(value), label: value < 1 ? `${value * 1000} mV/div` : `${value} V/div` }));
export const SAMPLE_RATES = [0.01, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100, 125].map(value => ({ value: String(value), label: value < 1 ? `${value * 1000} kS/s` : `${value} MS/s` }));
export const WINDOW_LENGTHS = [0.1, 0.5, 1, 5, 10, 25, 50, 100, 250, 500, 1000, 2000].map(value => ({ value: String(value), label: value < 1000 ? `${value} ms` : `${value / 1000} s` }));
