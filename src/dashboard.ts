// Dashboard de uso do Claude Code, renderizado na webview nativa.
// Os dados vêm do backend Rust pelo comando IPC `get_stats` (antes era um
// fetch a um servidor HTTP local). A lógica de render é a mesma do painel
// original (cards, heatmap, gráfico de modelos), só tipada em TypeScript.
import { invoke } from "@tauri-apps/api/core";
import { escapeHtml } from "./usage-format";

interface ModelTotals {
  in: number;
  out: number;
  cacheRead: number;
  cacheCreate: number;
}
interface ProjStats {
  msgs: number;
  tokens: number;
}
interface DayStats {
  date: string;
  msgs: number;
  userMsgs: number;
  tools: number;
  sessions: number;
  models: Record<string, ModelTotals>;
  toolsByName?: Record<string, number>;
  projects?: Record<string, ProjStats>;
}
interface SessionEntry {
  date: string;
  hour: number | null;
}
interface Baseline {
  lastComputedDate: string;
  hourCounts: Record<string, number>;
}
interface Stats {
  generatedAt: string;
  parseMs: number;
  files: number;
  reparsed: number;
  baseline: Baseline | null;
  days: DayStats[];
  sessions: SessionEntry[];
  error?: string;
}
interface ModelTotal {
  model: string;
  name: string;
  in: number;
  out: number;
  total: number;
}
interface ChartSeries {
  date: string;
  rows: { name: string; color: string; val: number }[];
}

// Paleta categórica de matizes distintos (azul, terracota, verde, roxo, âmbar,
// ciano, rosa, cinza) — antes era um degradê de azul, com pouco contraste entre
// modelos adjacentes no gráfico empilhado.
const PALETTE = ["#4c8dff", "#e0816f", "#7fc99a", "#c9a0f0", "#e6c34a", "#5bd1d4", "#f08fb0", "#9aa0a6"];
const MONTHS = ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."];

let DATA: Stats | null = null;
let range = "30";
let customFrom = "";
let customTo = "";
let customOpen = false; // popover de datas aberto (independe do range aplicado)
let tab = "geral";
let CHART_SERIES: ChartSeries[] = [];

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;

function friendly(model: string): string {
  // Ids novos: claude-opus-4-8 → "Opus 4.8".
  let m = model.match(/claude-(opus|sonnet|haiku)-(\d+)-(\d+)/);
  if (m) {
    const fam = m[1][0].toUpperCase() + m[1].slice(1);
    return fam + " " + m[2] + "." + m[3];
  }
  // Ids antigos: claude-3-5-sonnet-… → "Sonnet 3.5".
  m = model.match(/claude-(\d+)-(\d+)-(opus|sonnet|haiku)/);
  if (m) {
    const fam = m[3][0].toUpperCase() + m[3].slice(1);
    return fam + " " + m[1] + "." + m[2];
  }
  return model;
}
function abbr(n: number): string {
  if (n >= 1e9) return (n / 1e9).toFixed(1) + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "k";
  return String(Math.round(n));
}
const int = (n: number) => n.toLocaleString("pt-BR");
const dkey = (d: Date) =>
  d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") + "-" + String(d.getDate()).padStart(2, "0");
function dayLabel(dateStr: string): string {
  const d = new Date(dateStr + "T12:00:00");
  return d.getDate() + " de " + MONTHS[d.getMonth()];
}
// "YYYY-MM-DD" → "DD/MM/AA" (usado no rótulo do botão "Personalizado").
function fmtShort(key: string): string {
  const [y, m, d] = key.split("-");
  return d + "/" + m + "/" + y.slice(2);
}
const CUSTOM_LABEL = "Personalizado";

// Range personalizado normalizado (inverte se "de" > "até"); strings vazias =
// limite aberto.
function customRange(): { from: string; to: string } {
  let from = customFrom;
  let to = customTo;
  if (from && to && from > to) [from, to] = [to, from];
  return { from, to };
}

function filtered(): { days: DayStats[]; sessions: SessionEntry[] } {
  let days = DATA!.days;
  let sessions = DATA!.sessions;
  if (range === "custom") {
    const { from, to } = customRange();
    if (from) { days = days.filter((d) => d.date >= from); sessions = sessions.filter((s) => s.date >= from); }
    if (to) { days = days.filter((d) => d.date <= to); sessions = sessions.filter((s) => s.date <= to); }
  } else {
    const cut = new Date();
    cut.setDate(cut.getDate() - (Number(range) - 1));
    const cutKey = dkey(cut);
    days = days.filter((d) => d.date >= cutKey);
    sessions = sessions.filter((s) => s.date >= cutKey);
  }
  return { days, sessions };
}

function streaks(days: DayStats[]): { current: number; longest: number } {
  const active = new Set(days.filter((d) => d.msgs > 0 || d.sessions > 0).map((d) => d.date));
  let longest = 0, run = 0;
  // Meio-dia local (como heatmap/gráfico) para evitar o off-by-one de parsear
  // "YYYY-MM-DD" como meia-noite UTC em fusos negativos.
  const today = new Date(dkey(new Date()) + "T12:00:00");
  for (let d = new Date((days[0]?.date ?? dkey(today)) + "T12:00:00"); d <= today; d.setDate(d.getDate() + 1)) {
    if (active.has(dkey(d))) { run++; longest = Math.max(longest, run); } else run = 0;
  }
  // sequência atual: termina hoje (ou ontem, se hoje ainda sem uso)
  let current = 0;
  const probe = new Date();
  if (!active.has(dkey(probe))) probe.setDate(probe.getDate() - 1);
  while (active.has(dkey(probe))) { current++; probe.setDate(probe.getDate() - 1); }
  return { current, longest };
}

function modelTotals(days: DayStats[]): ModelTotal[] {
  const totals: Record<string, { in: number; out: number }> = {};
  for (const d of days) for (const [m, v] of Object.entries(d.models)) {
    const t = (totals[m] ??= { in: 0, out: 0 });
    t.in += v.in; t.out += v.out;
  }
  return Object.entries(totals)
    .map(([model, v]) => ({ model, name: friendly(model), in: v.in, out: v.out, total: v.in + v.out }))
    .sort((a, b) => b.total - a.total);
}

function renderCards(days: DayStats[], sessions: SessionEntry[]): void {
  const models = modelTotals(days);
  const totalTokens = models.reduce((s, m) => s + m.total, 0);
  const activeDays = days.filter((d) => d.msgs > 0 || d.sessions > 0).length;
  const st = streaks(days);
  const hours: Record<string, number> = {};
  for (const s of sessions) if (s.hour !== null) hours[s.hour] = (hours[s.hour] ?? 0) + 1;
  const peak = Object.entries(hours).sort((a, b) => b[1] - a[1])[0]?.[0] ?? "–";
  const cards: [string, string][] = [
    ["Sessões", int(sessions.length)],
    ["Mensagens", int(days.reduce((s, d) => s + d.msgs, 0))],
    ["Total de tokens", abbr(totalTokens)],
    ["Dias ativos", int(activeDays)],
    ["Sequência atual", st.current + "d"],
    ["Maior sequência", st.longest + "d"],
    ["Horário de pico", peak],
    ["Modelo favorito", escapeHtml(models[0]?.name ?? "–")],
  ];
  el("cards").innerHTML = cards
    .map(([l, v]) => '<div class="card"><div class="lbl">' + l + '</div><div class="val">' + v + "</div></div>")
    .join("");
}

function heatColor(v: number, max: number): string {
  if (!v) return "#2e2d29";
  const steps = ["#1d3a6e", "#24509e", "#2f6fed", "#5b8df2", "#86abf6"];
  const idx = Math.min(steps.length - 1, Math.floor((v / max) * steps.length));
  return steps[idx];
}

function renderHeat(days: DayStats[]): void {
  const byDate = new Map(days.map((d) => [d.date, d]));
  const max = Math.max(1, ...days.map((d) => d.msgs));
  const end = new Date(dkey(new Date()) + "T12:00:00"); // hoje ao meio-dia: garante a célula do dia atual
  // mínimo de 26 semanas (igual ao app oficial, que deixa colunas vazias à esquerda)
  const MIN_WEEKS = 26;
  const first = new Date((days[0]?.date ?? dkey(end)) + "T12:00:00");
  const minStart = new Date(end);
  minStart.setDate(minStart.getDate() - (MIN_WEEKS * 7 - 1));
  const start = first < minStart ? first : minStart;
  start.setDate(start.getDate() - start.getDay()); // alinhar no domingo
  const cells: string[] = [];
  for (let d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
    const key = dkey(d);
    const data = byDate.get(key);
    const v = data ? data.msgs : 0;
    cells.push('<div class="cell" style="background:' + heatColor(v, max) + '" data-tip="' +
      dayLabel(key) + " — " + int(v) + '"></div>');
  }
  el("heat").innerHTML = cells.join("");
}

// ----- tooltips -----
function placeFixed(node: HTMLElement, x: number, y: number): void {
  const r = node.getBoundingClientRect();
  let px = x + 14, py = y - r.height - 10;
  if (px + r.width > innerWidth - 8) px = x - r.width - 14;
  if (py < 8) py = y + 16;
  node.style.left = px + "px";
  node.style.top = py + "px";
}

function renderChart(days: DayStats[]): void {
  const models = modelTotals(days);
  const color = new Map(models.map((m, i) => [m.model, PALETTE[i % PALETTE.length]]));
  const W = 820, H = 260, padL = 52, padB = 26, padT = 10;
  // Span do eixo X: no range personalizado, fim/início = datas escolhidas; nos
  // presets, fim = hoje (garante a barra do dia atual) e início = 1º dia com dados.
  const cr = range === "custom" ? customRange() : { from: "", to: "" };
  const end = new Date((cr.to || dkey(new Date())) + "T12:00:00");
  const start = new Date((cr.from || days[0]?.date || dkey(end)) + "T12:00:00");
  const series: { date: string; models: Record<string, ModelTotals> }[] = [];
  const byDate = new Map(days.map((d) => [d.date, d]));
  for (let d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
    const key = dkey(d);
    series.push({ date: key, models: byDate.get(key)?.models ?? {} });
  }
  const dayTotal = (s: { models: Record<string, ModelTotals> }) =>
    Object.values(s.models).reduce((t, v) => t + v.in + v.out, 0);
  const maxY = Math.max(1, ...series.map(dayTotal));
  const innerW = W - padL - 8;
  const step = innerW / series.length;
  const bw = Math.max(2, Math.min(22, step - 3));
  let axis = "", bars = "", bands = "";

  CHART_SERIES = series.map((s) => ({
    date: s.date,
    rows: Object.entries(s.models)
      .map(([m, v]) => ({ name: friendly(m), color: color.get(m) ?? "#6e7681", val: v.in + v.out }))
      .filter((r) => r.val > 0)
      .sort((a, b) => b.val - a.val),
  }));

  // eixo Y (4 ticks)
  for (let t = 1; t <= 4; t++) {
    const v = (maxY / 4) * t;
    const y = H - padB - (v / maxY) * (H - padB - padT);
    axis += '<text x="' + (padL - 8) + '" y="' + (y + 4) + '" text-anchor="end">' + abbr(v) + "</text>";
    axis += '<line x1="' + padL + '" x2="' + W + '" y1="' + y + '" y2="' + y + '" stroke="#34322d" stroke-width="1"/>';
  }
  series.forEach((s, i) => {
    let y = H - padB;
    const x = padL + i * step + (step - bw) / 2;
    const order = (m: string) => { const idx = models.findIndex((mm) => mm.model === m); return idx < 0 ? 99 : idx; };
    // base da pilha = modelo com mais tokens no range (igual ao app)
    const entries = Object.entries(s.models).sort((a, b) => order(a[0]) - order(b[0]));
    for (const [m, v] of entries) {
      const val = v.in + v.out;
      if (!val) continue;
      const h = (val / maxY) * (H - padB - padT);
      y -= h;
      bars += '<rect x="' + x + '" y="' + y + '" width="' + bw + '" height="' + h + '" rx="1.5" fill="' + (color.get(m) ?? "#6e7681") + '"/>';
    }
    if (dayTotal(s) > 0) {
      bands += '<rect class="band" data-i="' + i + '" x="' + (padL + i * step) + '" y="' + padT +
        '" width="' + step + '" height="' + (H - padT - padB) + '" fill="transparent"/>';
    }
  });
  // eixo X (~6 labels)
  const every = Math.max(1, Math.floor(series.length / 6));
  series.forEach((s, i) => {
    if (i % every !== 0) return;
    axis += '<text x="' + (padL + i * step) + '" y="' + (H - 8) + '">' + dayLabel(s.date) + "</text>";
  });

  el("chart").innerHTML =
    '<svg id="chartsvg" viewBox="0 0 ' + W + " " + H + '" style="width:100%;margin-top:14px">' +
    axis + '<rect id="hl" fill="rgba(255,255,255,0.06)" rx="3" visibility="hidden"/>' + bars + bands + "</svg>";

  const svg = el("chartsvg");
  const hl = el("hl");
  const tip = el("tip");
  svg.addEventListener("mousemove", (e) => {
    const band = (e.target as HTMLElement).closest(".band");
    if (!band) { tip.classList.add("hide"); hl.setAttribute("visibility", "hidden"); return; }
    const i = Number((band as HTMLElement).dataset.i);
    const s = CHART_SERIES[i];
    hl.setAttribute("x", band.getAttribute("x")!);
    hl.setAttribute("y", band.getAttribute("y")!);
    hl.setAttribute("width", band.getAttribute("width")!);
    hl.setAttribute("height", band.getAttribute("height")!);
    hl.setAttribute("visibility", "visible");
    tip.innerHTML = '<div class="th">' + dayLabel(s.date) + "</div>" +
      s.rows.map((r) => '<div class="tr"><span class="dot" style="background:' + r.color + '"></span>' +
        escapeHtml(r.name) + "<b>" + abbr(r.val) + "</b></div>").join("");
    tip.classList.remove("hide");
    placeFixed(tip, e.clientX, e.clientY);
  });
  svg.addEventListener("mouseleave", () => { tip.classList.add("hide"); hl.setAttribute("visibility", "hidden"); });

  const grand = models.reduce((s, m) => s + m.total, 0) || 1;
  el("legend").innerHTML = models.map((m, i) =>
    '<div class="lrow"><div class="dot" style="background:' + PALETTE[i % PALETTE.length] + '"></div>' +
    "<div>" + escapeHtml(m.name) + "</div>" +
    '<div class="io">' + abbr(m.in) + " in · " + abbr(m.out) + " out</div>" +
    '<div class="pct">' + ((m.total / grand) * 100).toFixed(1) + "%</div></div>"
  ).join("");
}

// Abre/fecha o popover de datas (só a visibilidade; o range aplicado continua).
function setCustomOpen(open: boolean): void {
  customOpen = open;
  el("range-custom").classList.toggle("hide", !open);
}

// Uma linha do ranking horizontal (rótulo · barra proporcional · valor).
function rankRow(label: string, val: number, max: number, i: number, valText: string): string {
  const pct = ((val / max) * 100).toFixed(1);
  const color = PALETTE[i % PALETTE.length];
  return '<div class="rrow"><div class="rlabel" title="' + escapeHtml(label) + '">' + escapeHtml(label) +
    '</div><div class="rbar"><span style="width:' + pct + "%;background:" + color + '"></span></div>' +
    '<div class="rval">' + valText + "</div></div>";
}

const EMPTY_NOTE =
  '<div class="rank-empty">Sem dados no período (esta visão usa só os transcripts dos últimos ~30 dias).</div>';

function renderTools(days: DayStats[]): void {
  const totals: Record<string, number> = {};
  for (const d of days) for (const [name, c] of Object.entries(d.toolsByName ?? {})) totals[name] = (totals[name] ?? 0) + c;
  const rows = Object.entries(totals).sort((a, b) => b[1] - a[1]);
  const max = Math.max(1, ...rows.map((r) => r[1]));
  el("tools-rank").innerHTML = rows.length
    ? rows.map(([name, c], i) => rankRow(name, c, max, i, int(c))).join("")
    : EMPTY_NOTE;
}

function renderProjects(days: DayStats[]): void {
  const totals: Record<string, ProjStats> = {};
  for (const d of days) for (const [name, v] of Object.entries(d.projects ?? {})) {
    const acc = (totals[name] ??= { msgs: 0, tokens: 0 });
    acc.msgs += v.msgs; acc.tokens += v.tokens;
  }
  const rows = Object.entries(totals).sort((a, b) => b[1].tokens - a[1].tokens);
  const max = Math.max(1, ...rows.map((r) => r[1].tokens));
  el("proj-rank").innerHTML = rows.length
    ? rows.map(([name, v], i) => rankRow(name, v.tokens, max, i, abbr(v.tokens) + " · " + int(v.msgs) + " msgs")).join("")
    : EMPTY_NOTE;
}

function render(): void {
  if (!DATA) return;
  el("range-custom").classList.toggle("hide", !customOpen);
  const f = filtered();
  el("view-geral").classList.toggle("hide", tab !== "geral");
  el("view-modelos").classList.toggle("hide", tab !== "modelos");
  el("view-ferramentas").classList.toggle("hide", tab !== "ferramentas");
  el("view-projetos").classList.toggle("hide", tab !== "projetos");
  if (tab === "geral") { renderCards(f.days, f.sessions); renderHeat(f.days); }
  else if (tab === "modelos") renderChart(f.days);
  else if (tab === "ferramentas") renderTools(f.days);
  else if (tab === "projetos") renderProjects(f.days);
  el("foot").textContent = "Atualizado " + new Date(DATA.generatedAt).toLocaleTimeString("pt-BR");
}

function setOn(sel: string, target: EventTarget | null): void {
  document.querySelectorAll(sel + " button").forEach((b) => b.classList.toggle("on", b === target));
}

/// Busca os dados pelo IPC e re-renderiza. Barata em chamadas repetidas (o
/// backend mantém cache por arquivo), então pode ser chamada ao reabrir a view.
export async function loadDashboard(): Promise<void> {
  try {
    DATA = await invoke<Stats>("get_stats");
  } catch (e) {
    el("foot").textContent = "Falha ao carregar dados: " + (e instanceof Error ? e.message : String(e));
    return;
  }
  if (DATA.error) {
    el("foot").textContent = "Erro: " + DATA.error;
    return;
  }
  render();
}

// Pré-preenche os campos do popover (últimos 30 dias) quando vazios — só os
// VALORES dos inputs; nada é filtrado até clicar em "Aplicar". Também limita o
// seletor ao intervalo com dados: do dia mais antigo disponível até hoje.
function prefillCustomInputs(): void {
  const from = el("range-from") as HTMLInputElement;
  const to = el("range-to") as HTMLInputElement;
  const min = DATA?.days[0]?.date ?? "";
  const max = dkey(new Date());
  from.min = to.min = min;
  from.max = to.max = max;
  if (!to.value) to.value = max;
  if (!from.value) {
    const d = new Date();
    d.setDate(d.getDate() - 29);
    let f = dkey(d);
    if (min && f < min) f = min; // não sugerir antes do 1º dia com dados
    from.value = f;
  }
}

// "Aplicar" só habilita com as duas datas preenchidas.
function updateApplyState(): void {
  const from = el("range-from") as HTMLInputElement;
  const to = el("range-to") as HTMLInputElement;
  (el("range-apply") as HTMLButtonElement).disabled = !(from.value && to.value);
}

let initialized = false;

/// Liga os eventos da view do dashboard (uma vez) e dispara o primeiro load.
export function initDashboard(): void {
  if (initialized) { void loadDashboard(); return; }
  initialized = true;

  el("tab-geral").onclick = (e) => { tab = "geral"; setOn(".tabs", e.target); render(); };
  el("tab-modelos").onclick = (e) => { tab = "modelos"; setOn(".tabs", e.target); render(); };
  el("tab-ferramentas").onclick = (e) => { tab = "ferramentas"; setOn(".tabs", e.target); render(); };
  el("tab-projetos").onclick = (e) => { tab = "projetos"; setOn(".tabs", e.target); render(); };
  const customBtn = document.querySelector('.ranges button[data-r="custom"]') as HTMLButtonElement;

  document.querySelectorAll(".ranges button").forEach((b) =>
    (b as HTMLButtonElement).onclick = () => {
      const r = (b as HTMLElement).dataset.r!;
      // "Personalizado" só abre/fecha o popover; o filtro só vale ao "Aplicar".
      if (r === "custom") {
        prefillCustomInputs();
        updateApplyState();
        setCustomOpen(!customOpen);
        return;
      }
      // Preset (30d/7d): "desaplica" o range personalizado e restaura o rótulo.
      range = r;
      customBtn.textContent = CUSTOM_LABEL;
      setOn(".ranges", b);
      setCustomOpen(false);
      render();
    });

  const from = el("range-from") as HTMLInputElement;
  const to = el("range-to") as HTMLInputElement;
  from.oninput = updateApplyState;
  to.oninput = updateApplyState;

  (el("range-apply") as HTMLButtonElement).onclick = () => {
    if (!(from.value && to.value)) return;
    customFrom = from.value;
    customTo = to.value;
    range = "custom";
    // O botão "Personalizado" passa a exibir o range escolhido.
    const { from: f, to: t } = customRange();
    customBtn.textContent = fmtShort(f) + " – " + fmtShort(t);
    setOn(".ranges", customBtn);
    setCustomOpen(false);
    render();
  };

  // Fecha o popover ao clicar fora (exceto no próprio botão "Personalizado") ou
  // ao apertar Esc. O range escolhido permanece aplicado.
  document.addEventListener("mousedown", (e) => {
    if (!customOpen) return;
    const t = e.target as Node;
    const pop = el("range-custom");
    const btn = document.querySelector('.ranges button[data-r="custom"]');
    if (pop.contains(t) || btn?.contains(t)) return;
    setCustomOpen(false);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && customOpen) setCustomOpen(false);
  });

  const heat = el("heat");
  const pill = el("pill");
  heat.addEventListener("mousemove", (e) => {
    const t = (e.target as HTMLElement).closest(".cell") as HTMLElement | null;
    if (!t) { pill.classList.add("hide"); return; }
    pill.textContent = t.dataset.tip ?? "";
    pill.classList.remove("hide");
    placeFixed(pill, e.clientX, e.clientY);
  });
  heat.addEventListener("mouseleave", () => pill.classList.add("hide"));

  void loadDashboard();
}