import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

// Generate a classic CAN frame with CRC-15 and bit stuffing, then use the
// installed sigrok decoder to provide real annotations for our simulated UI.
const output = mkdtempSync(join(tmpdir(), 'analog-logic-scope-'));
const bitRate = 500000, rate = 20000000;
const bitsOf = (value, width) => Array.from({ length: width }, (_, i) => value >> (width - i - 1) & 1);
const payload = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
const message = [0, ...bitsOf(0x123, 11), 0, 0, 0, ...bitsOf(payload.length, 4), ...payload.flatMap(value => bitsOf(value, 8))];
let crc = 0;
for (const bit of message) {
  const feedback = (crc >> 14) ^ bit;
  crc = (crc << 1) & 0x7fff;
  if (feedback) crc ^= 0x4599;
}
const stuffed = [];
let previous = -1, run = 0;
for (const bit of [...message, ...bitsOf(crc, 15)]) {
  stuffed.push(bit);
  run = bit === previous ? run + 1 : 1;
  previous = bit;
  if (run === 5) { previous = 1 - bit; stuffed.push(previous); run = 1; }
}
const bits = [...Array(11).fill(1), ...stuffed, 1, 0, 1, ...Array(10).fill(1)];
const periodSamples = 10000;
const logic = Uint8Array.from({ length: periodSamples }, (_, i) => {
  const bit = bits[Math.floor(i / (rate / bitRate))] ?? 1;
  return bit | ((1 - bit) << 1);
});
const input = join(output, 'can.bin');
writeFileSync(input, logic);
const decoded = JSON.parse(execFileSync(process.env.SIGROK_CLI ?? resolve('.tools/sigrok-cli/sigrok-cli.exe'), [
  '-i', input, '-I', `binary:numchannels=2:samplerate=${rate}`, '-P', `can:can_rx=0:nominal_bitrate=${bitRate}`, '--protocol-decoder-jsontrace',
], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024, windowsHide: true }));
const annotations = [], pending = new Map();
for (const event of decoded.traceEvents) {
  if (event.ph === 'B') pending.set(event.tid, event);
  else if (event.ph === 'E' && pending.has(event.tid)) {
    const start = pending.get(event.tid);
    if (!/bits/i.test(event.tid)) annotations.push({ start_s: start.ts / 1e6, end_s: event.ts / 1e6, row: event.tid, text: start.name, value: null });
    pending.delete(event.tid);
  }
}
assert(annotations.some(a => /Identifier: 291|Identifier: 0x123/i.test(a.text)), 'CAN identifier decoded');
assert(!annotations.some(a => /warning|error/i.test(a.row)), 'Simulated frame must decode without warnings');
assert.equal(annotations.filter(a => /Data byte \d+:/.test(a.text)).length, 8, 'Eight payload bytes decoded');
console.log(`Decoded CAN ID 0x123, ${payload.map(v => v.toString(16)).join(' ')}, no warnings.`);

let server;
try { await fetch('http://127.0.0.1:1420'); }
catch {
  server = spawn(process.execPath, [resolve('node_modules/vite/bin/vite.js'), '--host', '127.0.0.1'], { windowsHide: true, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    await new Promise(resolve => setTimeout(resolve, 200));
    try { await fetch('http://127.0.0.1:1420'); break; } catch { /* wait for Vite */ }
  }
}
const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  for (const dpr of [1, 2]) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    // Test-only instrumentation of the served module, never the application bundle.
    await page.route(/\/src\/app\/ScopeTrack\.tsx(?:\?|$)/, async route => {
      const response = await route.fetch();
      const body = (await response.text()).replace('plot.current = chart;', 'plot.current = chart; (window.__plots ??= []).push(chart);');
      await route.fulfill({ response, body });
    });
    await page.addInitScript(({ bits, annotations, rate, bitRate, periodSamples }) => {
      let running = false, sequence = 0;
      window.__calls = {};
      window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
        window.__calls[command] = (window.__calls[command] ?? 0) + 1;
        if (command === 'discover_ad3') return [{ index: 0, name: 'Simulated CAN', serial: 'SIMULATION', in_use: false }];
        if (command === 'available_decoders') return [{ id: 'can', name: 'CAN' }, { id: 'uart', name: 'UART' }];
        if (command === 'decoder_details') return 'Simulated CAN-L on A1; CAN-H on A2';
        if (command === 'start_stream') { running = true; return; }
        if (command === 'stop_stream') { running = false; return; }
        if (command === 'decode_stream') return { origin_s: 0, results: [
          { Ok: { annotations, transitions: [80, 80], directory: 'simulation', origin_s: 0 } }, { Err: 'Decoder off' },
        ] };
        if (command === 'stream_snapshot') {
          sequence++;
          const gap = running && sequence % 3 === 0;
          const first = Math.floor((args.startS ?? 0) * rate);
          const count = Math.min(2048, args.pixels);
          const step = Math.max(1, Math.ceil((args.spanS ?? 0.1) * rate / count));
          const envelopes = [0, 1].map(channel => {
            const min = [], max = [];
            for (let bucket = 0; bucket < count; bucket++) {
              let lo = Infinity, hi = -Infinity;
              // Evaluate every sample: the fixture represents two million raw
              // samples; only the pixel-bounded min/max envelope crosses IPC.
              for (let i = 0; i < step; i++) {
                const sample = (first + bucket * step + i) % periodSamples;
                const bit = bits[Math.floor(sample / (rate / bitRate))] ?? 1;
                const value = 2.5 + (bit ? 0 : channel ? 1 : -1) + Math.sin(sequence) * 0.005;
                lo = Math.min(lo, value); hi = Math.max(hi, value);
              }
              min.push(lo); max.push(hi);
            }
            return { start_s: first / rate, step_s: step / rate, min, max };
          });
          return { running, directory: 'simulation', sample_rate_hz: rate, adc_bits: 14,
            total_samples: 2000000, sample_count: gap ? 0 : 2000000, samples_lost: gap ? 100 : 0, samples_corrupt: gap ? 20 : 0,
            origin_s: 0, duration_s: 0.1, sequence, error: null,
            envelopes: gap ? [{ start_s: 0, step_s: 0, min: [], max: [] }, { start_s: 0, step_s: 0, min: [], max: [] }] : envelopes,
            stats: [{ min_v: 1.5, max_v: 2.5, mean_v: 2 }, { min_v: 2.5, max_v: 3.5, mean_v: 3 }] };
        }
        throw new Error(`Unexpected command: ${command}`);
      } };
    }, { bits, annotations, rate, bitRate, periodSamples });
    await page.goto('http://127.0.0.1:1420');
    await page.getByRole('combobox', { name: 'A1 protocol', exact: true }).click();
    await page.getByRole('option', { name: 'CAN / CAN FD', exact: true }).click();
    await page.getByRole('button', { name: 'A1 bit rate', exact: true }).click();
    await page.getByRole('menuitemradio', { name: /^500\D000$/ }).click();
    await page.getByRole('combobox', { name: 'A2 protocol', exact: true }).click();
    await page.getByRole('option', { name: 'Off', exact: true }).click();
    await page.getByRole('button', { name: 'Start stream', exact: true }).click();
    await page.waitForFunction(() => window.__plots?.slice(-2).every(p => p.data[0].length > 0 && p.scales.y.min !== null));
    const checkPlot = async () => {
      const results = await page.evaluate(() => window.__plots.slice(-2).map(p => {
        const canvas = p.ctx.canvas, b = p.bbox;
        const { data } = p.ctx.getImageData(Math.ceil(b.left + 2), Math.ceil(b.top + 2), Math.floor(b.width - 4), Math.floor(b.height - 4));
        let ink = 0;
        for (let i = 0; i < data.length; i += 4) if (data[i + 3] > 100 && data[i] < 130) ink++;
        const over = p.over.getBoundingClientRect(), host = p.root.closest('.scope-track').getBoundingClientRect();
        return { points: p.data[0].length, min: p.scales.y.min, max: p.scales.y.max, ink,
          plotWidth: over.width, left: over.left - host.left, width: p.width, height: p.height };
      }));
      for (const p of results) {
        assert(p.min !== null && p.max > p.min, 'Finite voltage range after live update');
        assert(p.points <= 2048, 'Large capture stays bounded to display pixels');
        assert(p.ink > 500, `Waveform painted (${p.ink} dark pixels)`);
        assert(Math.abs(p.left - 62) < 1, 'Voltage axis has room');
      }
      return results;
    };
    await checkPlot();
    await page.getByRole('button', { name: 'View controls', exact: true }).click();
    await page.getByRole('menuitemradio', { name: 'Window zoom', exact: true }).click();
    const dragOver = await page.locator('.u-over').first().boundingBox();
    const dragSpan = await page.evaluate(() => { const p = window.__plots.at(-1); return p.scales.x.max - p.scales.x.min; });
    await page.mouse.move(dragOver.x + dragOver.width * 0.0001, dragOver.y + dragOver.height / 2);
    await page.mouse.down();
    await page.mouse.move(dragOver.x + dragOver.width * 0.05, dragOver.y + dragOver.height / 2, { steps: 4 });
    await page.mouse.up();
    await page.waitForFunction(oldSpan => { const p = window.__plots.at(-1); return p.scales.x.max - p.scales.x.min < oldSpan * 0.8; }, dragSpan);
    await page.waitForTimeout(400);
    await checkPlot(); // unchanged view with new data must remain visible
    await page.waitForSelector('.decode-count');
    assert.equal(await page.locator('.decode-bubble').count(), 0, 'Zoomed-out capture must not contain unreadable annotation boxes');
    const over = await page.locator('.u-over').first().boundingBox();
    await page.mouse.move(over.x - 8, over.y + over.height / 2);
    for (let i = 0; i < 12; i++) {
      const span = await page.evaluate(() => { const p = window.__plots.at(-1); return p.scales.x.max - p.scales.x.min; });
      if (span < 0.0003) break;
      await page.mouse.wheel(0, -500);
      await page.waitForFunction(oldSpan => { const p = window.__plots.at(-1); return p.scales.x.max - p.scales.x.min < oldSpan * 0.8; }, span, { timeout: 10000 }).catch(async error => {
        await page.screenshot({ path: join(output, `failed-dpr${dpr}.png`) });
        console.log({ dpr, i, span, over, output, current: await page.evaluate(() => window.__plots.slice(-2).map(p => ({ x: p.scales.x, over: p.over.getBoundingClientRect().toJSON() }))) });
        throw error;
      });
    }
    await page.waitForFunction(() => window.__plots.slice(-2).every(p => p.scales.x.max - p.scales.x.min < 0.0003));
    await page.waitForSelector('.decode-bubble');
    await checkPlot();
    const geometry = await page.evaluate(() => {
      const host = document.querySelector('.scope-track').getBoundingClientRect();
      const plot = document.querySelector('.u-over').getBoundingClientRect();
      const overlay = document.querySelector('.decode-overlays').getBoundingClientRect();
      return { plotLeft: plot.left - host.left, overlayLeft: overlay.left - host.left, plotWidth: plot.width, overlayWidth: overlay.width, plotTop: plot.top - host.top, overlayBottom: overlay.bottom - host.top };
    });
    assert(Math.abs(geometry.plotLeft - geometry.overlayLeft) < 1 && Math.abs(geometry.plotWidth - geometry.overlayWidth) < 1, 'Decode overlay alignment at this DPR');
    assert(geometry.overlayBottom <= geometry.plotTop + 1, 'Decode overlay stays outside waveform plot');
    assert(await page.locator('.decode-bubble').evaluateAll(elements => elements.every(e => e.scrollWidth <= e.clientWidth)), 'Visible annotations must fit without clipping');
    const annotation = page.locator('.decode-bubble').filter({ hasText: 'D0: 0x11' }).first();
    await annotation.hover();
    const highlight = await page.locator('.decode-highlight').boundingBox();
    assert(highlight && highlight.width > 0 && highlight.height >= geometry.plotTop - geometry.overlayBottom, 'Hovering an annotation highlights its decoded interval');
    await page.screenshot({ path: join(output, `can-dpr${dpr}.png`) });
    await annotation.click();
    await page.waitForFunction(() => window.__plots.at(-1).scales.x.max - window.__plots.at(-1).scales.x.min < 0.0001);
    await page.setViewportSize({ width: 1100, height: 760 });
    await page.waitForFunction(() => window.__plots.slice(-2).every(p => p.width < 900));
    await checkPlot();
    await page.getByRole('button', { name: 'Stop stream', exact: true }).click();
    await page.waitForTimeout(250);
    await checkPlot();
    const callsBeforeFormat = await page.evaluate(() => ({ ...window.__calls }));
    for (const [format, label] of [['Decimal', 'D0: 17'], ['Binary', 'D0: 00010001'], ['ASCII', 'D0: \\x11'], ['Hex', 'D0: 0x11']]) {
      await page.getByRole('combobox', { name: 'Decoded display format' }).click();
      await page.getByRole('option', { name: format, exact: true }).click();
      await page.locator('.decode-bubble').filter({ hasText: label }).first().waitFor();
      assert(await page.locator('.decode-bubble').evaluateAll(elements => elements.every(e => e.scrollWidth <= e.clientWidth)), 'Formatted annotations must fit');
    }
    assert.deepEqual(await page.evaluate(() => window.__calls), callsBeforeFormat, 'Format switching must reuse existing capture and annotations without IPC');
    await page.locator('.scope-track').first().dblclick();
    await page.waitForFunction(() => Math.abs(window.__plots.at(-1).scales.x.max - 0.1) < 0.001);
    assert(await page.evaluate(() => Math.abs(window.__plots.at(-1).scales.x.max - 0.1) < 0.001), 'Stopped view keeps the final live window');
    await page.screenshot({ path: join(output, `can-stopped-dpr${dpr}.png`) });
    const timings = await page.evaluate(() => {
      const samples = [];
      for (let i = 0; i < 60; i++) {
        const start = performance.now();
        for (const p of window.__plots.slice(-2)) p.batch(() => {
          p.setData([...p.data], false);
          p.setScale('x', { min: p.scales.x.min, max: p.scales.x.max });
          p.setScale('y', { min: p.scales.y.min, max: p.scales.y.max });
        });
        samples.push(performance.now() - start);
      }
      samples.sort((a, b) => a - b);
      return { median: samples[30], p95: samples[57] };
    });
    assert.deepEqual(errors, [], 'No browser runtime errors');
    console.log(`DPR ${dpr}: live data, pixel bounds, waveform pixels, zoom, resize, annotations, stop and format switching without IPC passed.`);
    console.log(`Two-chart update: median ${timings.median.toFixed(2)} ms, p95 ${timings.p95.toFixed(2)} ms (headless Edge, fixture only).`);
    await page.close();
  }
  console.log(`Screenshots and CAN fixture: ${output}`);
} finally {
  await browser.close();
  server?.kill();
}
