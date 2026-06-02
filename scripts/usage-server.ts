#!/usr/bin/env bun
// Mini-servidor local que replica o painel de uso do Claude Desktop.
// Fontes (as mesmas do painel oficial):
//   1. ~/.claude/projects/**/*.jsonl  — transcripts vivos (~últimos 30 dias; o cleanup apaga os antigos)
//   2. ~/.claude/stats-cache.json     — baseline histórico consolidado antes do cleanup
// Para bater com o Desktop, a contagem NÃO deduplica linhas repetidas de message.id
// (sessões retomadas copiam linhas pro novo arquivo) — é assim que o app oficial conta.
// O HTML é compartilhado com o tray agent: src-tauri/assets/usage-dashboard.html
// Rodar com: bun scripts/usage-server.ts  (http://localhost:3210)

import path from 'node:path';
import os from 'node:os';
import { stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const PORT = Number(process.env.USAGE_PORT ?? 3210);
const CLAUDE_DIR = path.join(os.homedir(), '.claude');
const PROJECTS_DIR = path.join(CLAUDE_DIR, 'projects');
const STATS_CACHE = path.join(CLAUDE_DIR, 'stats-cache.json');
const HTML_FILE = fileURLToPath(new URL('../src-tauri/assets/usage-dashboard.html', import.meta.url));

interface ModelDayTotals {
  in: number;
  out: number;
  cacheRead: number;
  cacheCreate: number;
}

interface DayStats {
  msgs: number;
  userMsgs: number;
  tools: number;
  sessions: number;
  models: Record<string, ModelDayTotals>;
}

interface TranscriptData {
  // agregados por dia deste arquivo (sem dedup, igual ao painel oficial)
  days: Map<string, DayStats>;
  sessions: Array<{ id: string; ts: string }>;
}

// cache: filePath -> { mtimeMs, size, data }
const fileCache = new Map<string, { mtimeMs: number; size: number; data: TranscriptData }>();

function localDate(ts: string): string | null {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return null;
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function emptyDay(): DayStats {
  return { msgs: 0, userMsgs: 0, tools: 0, sessions: 0, models: {} };
}

function parseTranscript(text: string): TranscriptData {
  const days = new Map<string, DayStats>();
  const firstTsBySession = new Map<string, string>();

  const day = (date: string): DayStats => {
    let d = days.get(date);
    if (!d) { d = emptyDay(); days.set(date, d); }
    return d;
  };

  for (const line of text.split('\n')) {
    if (!line) continue;
    let o: any;
    try { o = JSON.parse(line); } catch { continue; }
    const t = o.type;
    if (t !== 'user' && t !== 'assistant') continue;
    const ts: string | undefined = o.timestamp;
    const date = ts ? localDate(ts) : null;
    if (!date || !ts) continue;

    if (o.sessionId && !o.isSidechain && !firstTsBySession.has(o.sessionId)) {
      firstTsBySession.set(o.sessionId, ts);
    }

    const msg = o.message;
    if (t === 'user') {
      if (o.isMeta) continue;
      // respostas de tool chegam como "user" com content [{type:'tool_result'}] — não são mensagens humanas
      const content = msg?.content;
      const isToolResult = Array.isArray(content) && content.some((c: any) => c?.type === 'tool_result');
      if (!isToolResult) { const d = day(date); d.msgs++; d.userMsgs++; }
    } else {
      const d = day(date);
      d.msgs++;
      const content = msg?.content;
      if (Array.isArray(content)) d.tools += content.filter((c: any) => c?.type === 'tool_use').length;

      const u = msg?.usage;
      const model: string | undefined = msg?.model;
      if (u && model && !model.startsWith('<')) {
        const mm = (d.models[model] ??= { in: 0, out: 0, cacheRead: 0, cacheCreate: 0 });
        mm.in += u.input_tokens ?? 0;
        mm.out += u.output_tokens ?? 0;
        mm.cacheRead += u.cache_read_input_tokens ?? 0;
        mm.cacheCreate += u.cache_creation_input_tokens ?? 0;
      }
    }
  }

  const sessions = [...firstTsBySession.entries()].map(([id, ts]) => ({ id, ts }));
  return { days, sessions };
}

interface CacheBaseline {
  lastComputedDate: string;
  days: Map<string, DayStats>;
  hourCounts: Record<string, number>;
}

// Baseline do stats-cache.json: cobre o período cujos transcripts já foram apagados pelo cleanup.
async function loadCacheBaseline(): Promise<CacheBaseline | null> {
  const file = Bun.file(STATS_CACHE);
  if (!(await file.exists())) return null;
  let raw: any;
  try { raw = await file.json(); } catch { return null; }
  if (!raw?.lastComputedDate) return null;

  const days = new Map<string, DayStats>();
  const day = (date: string): DayStats => {
    let d = days.get(date);
    if (!d) { d = emptyDay(); days.set(date, d); }
    return d;
  };

  for (const a of raw.dailyActivity ?? []) {
    const d = day(a.date);
    d.msgs = a.messageCount ?? 0;
    d.tools = a.toolCallCount ?? 0;
    d.sessions = a.sessionCount ?? 0;
  }

  // dailyModelTokens traz só o total in+out por modelo/dia; o split in/out vem
  // da proporção global de modelUsage (a soma por modelo fecha exata com o painel)
  const ratioIn: Record<string, number> = {};
  for (const [model, u] of Object.entries<any>(raw.modelUsage ?? {})) {
    const total = (u.inputTokens ?? 0) + (u.outputTokens ?? 0);
    ratioIn[model] = total ? (u.inputTokens ?? 0) / total : 0;
  }
  for (const e of raw.dailyModelTokens ?? []) {
    const d = day(e.date);
    for (const [model, total] of Object.entries<any>(e.tokensByModel ?? {})) {
      const inTok = Math.round((total as number) * (ratioIn[model] ?? 0));
      d.models[model] = { in: inTok, out: (total as number) - inTok, cacheRead: 0, cacheCreate: 0 };
    }
  }

  return { lastComputedDate: raw.lastComputedDate, days, hourCounts: raw.hourCounts ?? {} };
}

async function collectStats() {
  const started = Date.now();
  const glob = new Bun.Glob('**/*.jsonl');
  const files: string[] = [];
  for await (const rel of glob.scan({ cwd: PROJECTS_DIR })) {
    files.push(path.join(PROJECTS_DIR, rel));
  }
  let parsed = 0;

  await Promise.all(files.map(async (file) => {
    const st = await stat(file).catch(() => null);
    if (!st) return;
    const cached = fileCache.get(file);
    if (cached && cached.mtimeMs === st.mtimeMs && cached.size === st.size) return;
    const text = await Bun.file(file).text().catch(() => '');
    fileCache.set(file, { mtimeMs: st.mtimeMs, size: st.size, data: parseTranscript(text) });
    parsed++;
  }));

  // remove do cache arquivos que sumiram
  const live = new Set(files);
  for (const key of fileCache.keys()) if (!live.has(key)) fileCache.delete(key);

  const baseline = await loadCacheBaseline();
  const cutoff = baseline?.lastComputedDate ?? '';

  // dias ≤ lastComputedDate vêm do baseline (consolidado); posteriores, dos transcripts
  const days = new Map<string, DayStats>();
  if (baseline) for (const [date, d] of baseline.days) days.set(date, d);

  const day = (date: string): DayStats => {
    let d = days.get(date);
    if (!d) { d = emptyDay(); days.set(date, d); }
    return d;
  };

  const sessionFirstTs = new Map<string, string>();
  for (const { data } of fileCache.values()) {
    for (const [date, src] of data.days) {
      if (date <= cutoff) continue; // já consolidado no baseline
      const d = day(date);
      d.msgs += src.msgs; d.userMsgs += src.userMsgs; d.tools += src.tools;
      for (const [model, v] of Object.entries(src.models)) {
        const mm = (d.models[model] ??= { in: 0, out: 0, cacheRead: 0, cacheCreate: 0 });
        mm.in += v.in; mm.out += v.out; mm.cacheRead += v.cacheRead; mm.cacheCreate += v.cacheCreate;
      }
    }
    for (const s of data.sessions) {
      const prev = sessionFirstTs.get(s.id);
      if (!prev || s.ts < prev) sessionFirstTs.set(s.id, s.ts);
    }
  }

  const sessions: Array<{ date: string; hour: number | null }> = [];
  for (const ts of sessionFirstTs.values()) {
    const date = localDate(ts);
    if (!date || date <= cutoff) continue; // sessões antigas já estão no baseline
    day(date).sessions++;
    sessions.push({ date, hour: new Date(ts).getHours() });
  }
  // sessões do baseline entram sem hora (a hora agregada vem de hourCounts)
  if (baseline) {
    for (const [date, d] of baseline.days) {
      for (let i = 0; i < d.sessions; i++) sessions.push({ date, hour: null });
    }
  }

  const daysArr = [...days.entries()]
    .map(([date, d]) => ({ date, ...d }))
    .sort((a, b) => a.date.localeCompare(b.date));

  return {
    generatedAt: new Date().toISOString(),
    parseMs: Date.now() - started,
    files: files.length,
    reparsed: parsed,
    baseline: baseline ? { lastComputedDate: baseline.lastComputedDate, hourCounts: baseline.hourCounts } : null,
    days: daysArr,
    sessions,
  };
}

Bun.serve({
  port: PORT,
  async fetch(req) {
    const { pathname } = new URL(req.url);
    if (pathname === '/api/stats') {
      return Response.json(await collectStats());
    }
    if (pathname === '/' || pathname === '/index.html') {
      const html = await Bun.file(HTML_FILE).text();
      return new Response(html, { headers: { 'content-type': 'text/html; charset=utf-8' } });
    }
    return new Response('not found', { status: 404 });
  },
});

console.log(`Dashboard de uso do Claude Code: http://localhost:${PORT}`);
console.log(`Lendo transcripts de: ${PROJECTS_DIR}`);
