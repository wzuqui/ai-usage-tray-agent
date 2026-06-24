// Dashboard de uso do Codex, renderizado na webview nativa. Espelha a estrutura
// da "Dashboard Claude" (cards + gráfico de barras empilhadas + tooltip), mas os
// dados vêm de uma chamada de rede (analytics do backend do ChatGPT) pelo comando
// IPC `get_codex_stats`, e a unidade é PERCENTUAL de uso diário (não tokens).
// A tela carrega ao abrir e refaz a chamada ao trocar o período (30d/7d).
import { invoke } from "@tauri-apps/api/core";
import { escapeHtml } from "./usage-format";

interface ModelUsage {
  model: string;
  speed: string;
  credits: number;
}
interface CodexDay {
  date: string;
  product_surface_usage_values: Record<string, number>;
  models: ModelUsage[];
}
interface CodexStats {
  units?: string;
  groupBy?: string;
  days: CodexDay[];
  generatedAt: string;
  error?: string;
}
interface Segment {
  key: string;
  label: string;
  color: string;
  val: number;
}
interface Series {
  date: string;
  segments: Segment[];
}

const PALETTE = ["#10a37f", "#2f6fed", "#5b8df2", "#86abf6", "#f2a35b", "#c77dff", "#8b949e", "#e06c75", "#56b6c2", "#d19a66", "#98c379", "#6e7681"];
const MONTHS = ["jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez."];

// Rótulos amigáveis das origens (product surfaces) retornadas pela API.
const SURFACE_LABELS: Record<string, string> = {
  cli: "CLI", vscode: "VS Code", web: "Web", slack: "Slack", linear: "Linear",
  jetbrains: "JetBrains", sdk: "SDK", exec: "Exec", github: "GitHub",
  desktop_app: "Desktop", github_code_review: "Code Review",
  agent_identity: "Agent", unknown: "Outros",
};

let DATA: CodexStats | null = null;
let days = 30; // janela atual (refaz a chamada ao trocar)
let tab = "geral"; // geral | surfaces | modelos
let loading = false;
let CHART_SERIES: Series[] = [];

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;

function surfaceLabel(key: string): string {
  return SURFACE_LABELS[key] ?? key;
}
function modelLabel(m: ModelUsage): string {
  const name = m.model.replace(/^gpt-/, "GPT-").replace(/^codex-/, "Codex ");
  return m.speed && m.speed !== "standard" ? `${name} (${m.speed})` : name;
}
function pct(n: number): string {
  if (n <= 0) return "0%";
  return (n < 10 ? n.toFixed(1) : Math.round(n).toString()) + "%";
}
function dayLabel(dateStr: string): string {
  const d = new Date(dateStr + "T12:00:00");
  return d.getDate() + " de " + MONTHS[d.getMonth()];
}
const dayTotal = (s: Series) => s.segments.reduce((t, seg) => t + seg.val, 0);

// ----- construção das séries por aba -----
function buildSeries(): Series[] {
  if (!DATA) return [];
  if (tab === "modelos") {
    const colorByKey = new Map<string, string>();
    let next = 0;
    return DATA.days.map((d) => ({
      date: d.date,
      segments: (d.models ?? [])
        .filter((m) => m.credits > 0)
        .map((m) => {
          const key = m.model + "|" + m.speed;
          if (!colorByKey.has(key)) colorByKey.set(key, PALETTE[next++ % PALETTE.length]);
          return { key, label: modelLabel(m), color: colorByKey.get(key)!, val: m.credits };
        })
        .sort((a, b) => b.val - a.val),
    }));
  }
  if (tab === "surfaces") {
    // cor estável por surface (ordem fixa da PALETTE pela ordem de SURFACE_LABELS)
    const order = Object.keys(SURFACE_LABELS);
    const colorByKey = (k: string) => PALETTE[Math.max(0, order.indexOf(k)) % PALETTE.length];
    return DATA.days.map((d) => ({
      date: d.date,
      segments: Object.entries(d.product_surface_usage_values ?? {})
        .filter(([, v]) => v > 0)
        .map(([k, v]) => ({ key: k, label: surfaceLabel(k), color: colorByKey(k), val: v }))
        .sort((a, b) => b.val - a.val),
    }));
  }
  // geral: total diário em um único segmento
  return DATA.days.map((d) => {
    const total = (d.models ?? []).reduce((t, m) => t + m.credits, 0);
    return {
      date: d.date,
      segments: total > 0 ? [{ key: "total", label: "Uso", color: PALETTE[0], val: total }] : [],
    };
  });
}

// agregados por chave (legenda + predominante)
function aggregate(series: Series[]): { key: string; label: string; color: string; total: number }[] {
  const acc = new Map<string, { label: string; color: string; total: number }>();
  for (const s of series) for (const seg of s.segments) {
    const e = acc.get(seg.key) ?? { label: seg.label, color: seg.color, total: 0 };
    e.total += seg.val;
    acc.set(seg.key, e);
  }
  return [...acc.entries()].map(([key, v]) => ({ key, ...v })).sort((a, b) => b.total - a.total);
}

function renderCards(series: Series[]): void {
  const totals = series.map(dayTotal);
  const active = totals.filter((t) => t > 0);
  const activeDays = active.length;
  const sum = totals.reduce((a, b) => a + b, 0);
  const avg = activeDays ? sum / activeDays : 0;
  let peakIdx = -1, peakVal = 0;
  totals.forEach((t, i) => { if (t > peakVal) { peakVal = t; peakIdx = i; } });

  // predominantes por surface e por modelo (independente da aba atual)
  const topSurface = aggregate(buildSeriesFor("surfaces"))[0];
  const topModel = aggregate(buildSeriesFor("modelos"))[0];

  const cards: [string, string][] = [
    ["Dias ativos", String(activeDays)],
    ["Uso médio/dia", pct(avg)],
    ["Dia de pico", peakIdx >= 0 ? dayLabel(series[peakIdx].date) : "–"],
    ["Maior uso", pct(peakVal)],
    ["Origem principal", escapeHtml(topSurface?.label ?? "–")],
    ["Modelo principal", escapeHtml(topModel?.label ?? "–")],
  ];
  el("codex-cards").innerHTML = cards
    .map(([l, v]) => '<div class="card"><div class="lbl">' + l + '</div><div class="val">' + v + "</div></div>")
    .join("");
}

// helper para os cards: constrói séries de uma aba específica sem mexer no estado
function buildSeriesFor(forTab: string): Series[] {
  const saved = tab; tab = forTab;
  const out = buildSeries();
  tab = saved;
  return out;
}

// ----- tooltip -----
function placeFixed(node: HTMLElement, x: number, y: number): void {
  const r = node.getBoundingClientRect();
  let px = x + 14, py = y - r.height - 10;
  if (px + r.width > innerWidth - 8) px = x - r.width - 14;
  if (py < 8) py = y + 16;
  node.style.left = px + "px";
  node.style.top = py + "px";
}

function renderChart(series: Series[]): void {
  CHART_SERIES = series;
  const W = 820, H = 260, padL = 52, padB = 26, padT = 10;
  const maxY = Math.max(1, ...series.map(dayTotal));
  const innerW = W - padL - 8;
  const step = innerW / Math.max(1, series.length);
  const bw = Math.max(2, Math.min(22, step - 3));
  let axis = "", bars = "", bands = "";

  // eixo Y (4 ticks, em %)
  for (let t = 1; t <= 4; t++) {
    const v = (maxY / 4) * t;
    const y = H - padB - (v / maxY) * (H - padB - padT);
    axis += '<text x="' + (padL - 8) + '" y="' + (y + 4) + '" text-anchor="end">' + Math.round(v) + "%</text>";
    axis += '<line x1="' + padL + '" x2="' + W + '" y1="' + y + '" y2="' + y + '" stroke="#34322d" stroke-width="1"/>';
  }
  series.forEach((s, i) => {
    let y = H - padB;
    const x = padL + i * step + (step - bw) / 2;
    for (const seg of s.segments) {
      if (seg.val <= 0) continue;
      const h = (seg.val / maxY) * (H - padB - padT);
      y -= h;
      bars += '<rect x="' + x + '" y="' + y + '" width="' + bw + '" height="' + h + '" rx="1.5" fill="' + seg.color + '"/>';
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

  el("codex-chart").innerHTML =
    '<svg id="codex-chartsvg" viewBox="0 0 ' + W + " " + H + '" style="width:100%;margin-top:14px">' +
    axis + '<rect id="codex-hl" fill="rgba(255,255,255,0.06)" rx="3" visibility="hidden"/>' + bars + bands + "</svg>";

  const svg = el("codex-chartsvg");
  const hl = el("codex-hl");
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
    tip.innerHTML = '<div class="th">' + dayLabel(s.date) + " — " + pct(dayTotal(s)) + "</div>" +
      s.segments.map((seg) => '<div class="tr"><span class="dot" style="background:' + seg.color + '"></span>' +
        escapeHtml(seg.label) + "<b>" + pct(seg.val) + "</b></div>").join("");
    tip.classList.remove("hide");
    placeFixed(tip, e.clientX, e.clientY);
  });
  svg.addEventListener("mouseleave", () => { tip.classList.add("hide"); hl.setAttribute("visibility", "hidden"); });

  // legenda (oculta na visão geral, que tem só um segmento)
  const legend = el("codex-legend");
  if (tab === "geral") { legend.innerHTML = ""; return; }
  const agg = aggregate(series);
  const grand = agg.reduce((a, b) => a + b.total, 0) || 1;
  legend.innerHTML = agg.map((a) =>
    '<div class="lrow"><div class="dot" style="background:' + a.color + '"></div>' +
    "<div>" + escapeHtml(a.label) + "</div>" +
    '<div class="io">' + pct(a.total) + " acum.</div>" +
    '<div class="pct">' + ((a.total / grand) * 100).toFixed(1) + "%</div></div>"
  ).join("");
}

function render(): void {
  if (!DATA) return;
  const series = buildSeries();
  el("codex-view-geral").classList.toggle("hide", tab !== "geral");
  if (tab === "geral") renderCards(series);
  renderChart(series);
  el("codex-foot").textContent =
    DATA.days.length + " dias · " + days + "d · atualizado " +
    new Date(DATA.generatedAt).toLocaleTimeString("pt-BR");
}

function setOn(sel: string, target: EventTarget | null): void {
  document.querySelectorAll(sel + " button").forEach((b) => b.classList.toggle("on", b === target));
}

/// Busca os dados pelo IPC (chamada de rede) e re-renderiza. Mostra um estado de
/// carregando porque, diferente do Claude, aqui há latência de rede.
export async function loadCodexDashboard(): Promise<void> {
  if (loading) return;
  loading = true;
  el("codex-foot").textContent = "Carregando…";
  try {
    DATA = await invoke<CodexStats>("get_codex_stats", { days });
  } catch (e) {
    el("codex-foot").textContent = "Falha ao carregar dados: " + (e instanceof Error ? e.message : String(e));
    return;
  } finally {
    loading = false;
  }
  if (DATA.error) {
    el("codex-foot").textContent = "Erro: " + DATA.error;
    el("codex-cards").innerHTML = "";
    el("codex-chart").innerHTML = "";
    el("codex-legend").innerHTML = "";
    return;
  }
  render();
}

let initialized = false;

/// Liga os eventos da view (uma vez) e dispara o primeiro load.
export function initCodexDashboard(): void {
  if (initialized) { void loadCodexDashboard(); return; }
  initialized = true;

  el("codex-tab-geral").onclick = (e) => { tab = "geral"; setOn(".codex-tabs", e.target); render(); };
  el("codex-tab-surfaces").onclick = (e) => { tab = "surfaces"; setOn(".codex-tabs", e.target); render(); };
  el("codex-tab-modelos").onclick = (e) => { tab = "modelos"; setOn(".codex-tabs", e.target); render(); };
  document.querySelectorAll(".codex-ranges button").forEach((b) =>
    ((b as HTMLButtonElement).onclick = () => {
      days = Number((b as HTMLElement).dataset.d) || 30;
      setOn(".codex-ranges", b);
      void loadCodexDashboard();
    }));

  void loadCodexDashboard();
}
