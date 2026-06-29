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
let days = 30; // janela do preset (refaz a chamada ao trocar)
let customFrom = "";
let customTo = "";
let customActive = false; // range personalizado aplicado (envia start/end)
let customOpen = false; // popover de datas aberto
let tab = "geral"; // geral | surfaces | modelos
let loading = false;
let CHART_SERIES: Series[] = [];

const CUSTOM_LABEL = "Personalizado";
const MAX_DAYS = 90; // limite da API do Codex

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;
const dkey = (d: Date): string =>
  d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") + "-" + String(d.getDate()).padStart(2, "0");
// "YYYY-MM-DD" → "DD/MM/AA" (rótulo do botão "Personalizado").
const fmtShort = (key: string): string => { const [y, m, d] = key.split("-"); return d + "/" + m + "/" + y.slice(2); };

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
// `forTab` permite construir as séries de uma aba específica (usado pelos cards
// de "predominantes") sem mexer no estado global `tab`; o padrão é a aba atual.
function buildSeries(forTab: string = tab): Series[] {
  if (!DATA) return [];
  if (forTab === "modelos") {
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
  if (forTab === "surfaces") {
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
  const topSurface = aggregate(buildSeries("surfaces"))[0];
  const topModel = aggregate(buildSeries("modelos"))[0];

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
    tip.innerHTML = '<div class="th">' + dayLabel(s.date) + "</div>" +
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
  el("codex-foot").textContent = "Atualizado " + new Date(DATA.generatedAt).toLocaleTimeString("pt-BR");
  // A API devolve dias zerados quando não houve uso; mostra um estado vazio
  // amigável em vez de um gráfico achatado.
  const hasUsage = DATA.days.some((d) =>
    (d.models ?? []).some((m) => m.credits > 0) ||
    Object.values(d.product_surface_usage_values ?? {}).some((v) => v > 0));
  el("codex-view-geral").classList.toggle("hide", tab !== "geral");
  if (!hasUsage) {
    renderMessage("Nenhum uso do Codex neste período.");
    return;
  }
  const series = buildSeries();
  if (tab === "geral") renderCards(series);
  renderChart(series);
}

// ----- range de data personalizado (popover) -----
function setCustomOpen(open: boolean): void {
  customOpen = open;
  el("codex-range-custom").classList.toggle("hide", !open);
}
// Range normalizado (inverte se "de" > "até").
function customRange(): { from: string; to: string } {
  let from = customFrom, to = customTo;
  if (from && to && from > to) [from, to] = [to, from];
  return { from, to };
}
// Pré-preenche os campos (últimos 30 dias) e limita o seletor à janela suportada
// pelo Codex: dos últimos 90 dias até hoje.
function prefillCustomInputs(): void {
  const from = el("codex-range-from") as HTMLInputElement;
  const to = el("codex-range-to") as HTMLInputElement;
  const minD = new Date(); minD.setDate(minD.getDate() - (MAX_DAYS - 1));
  const min = dkey(minD), max = dkey(new Date());
  from.min = to.min = min;
  from.max = to.max = max;
  if (!to.value) to.value = max;
  if (!from.value) {
    const d = new Date(); d.setDate(d.getDate() - 29);
    let f = dkey(d);
    if (f < min) f = min;
    from.value = f;
  }
}
function updateApplyState(): void {
  const from = el("codex-range-from") as HTMLInputElement;
  const to = el("codex-range-to") as HTMLInputElement;
  (el("codex-range-apply") as HTMLButtonElement).disabled = !(from.value && to.value);
}

function setOn(sel: string, target: EventTarget | null): void {
  document.querySelectorAll(sel + " button").forEach((b) => b.classList.toggle("on", b === target));
}

// Skeleton (shimmer) enquanto a chamada de rede não volta — substitui o antigo
// texto "Carregando…" no rodapé.
function renderLoading(): void {
  el("codex-cards").innerHTML = Array.from({ length: 6 }, () =>
    '<div class="card skel"><div class="skel-line lbl"></div><div class="skel-line val"></div></div>').join("");
  el("codex-chart").innerHTML = '<div class="codex-skel-chart"></div>';
  el("codex-legend").innerHTML = "";
}

// Mensagem ocupando a área do conteúdo (vazio ou erro).
function renderMessage(text: string, isError = false): void {
  el("codex-cards").innerHTML = "";
  el("codex-legend").innerHTML = "";
  el("codex-chart").innerHTML = '<div class="codex-empty' + (isError ? " err" : "") + '">' + escapeHtml(text) + "</div>";
}

/// Busca os dados pelo IPC (chamada de rede) e re-renderiza. Mostra um skeleton
/// durante a chamada porque, diferente do Claude, aqui há latência de rede.
export async function loadCodexDashboard(opts?: { skeleton?: boolean }): Promise<void> {
  if (loading) return;
  loading = true;
  // Skeleton só na 1ª carga ou em ação explícita (troca de período/aplicar range).
  // Em refresh de fundo (foco/resize da janela) mantém o conteúdo atual para não
  // piscar o skeleton.
  if (opts?.skeleton ?? !DATA) {
    renderLoading();
    el("codex-foot").textContent = "";
  }
  try {
    const range = customRange();
    const args = customActive ? { days, start: range.from, end: range.to } : { days };
    DATA = await invoke<CodexStats>("get_codex_stats", args);
  } catch (e) {
    renderMessage("Falha ao carregar dados: " + (e instanceof Error ? e.message : String(e)), true);
    return;
  } finally {
    loading = false;
  }
  if (DATA.error) {
    renderMessage(DATA.error, true);
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
  const customBtn = document.querySelector('.codex-ranges button[data-d="custom"]') as HTMLButtonElement;

  document.querySelectorAll(".codex-ranges button").forEach((b) =>
    ((b as HTMLButtonElement).onclick = () => {
      const d = (b as HTMLElement).dataset.d!;
      // "Personalizado" só abre/fecha o popover; o filtro vale ao "Aplicar".
      if (d === "custom") {
        prefillCustomInputs();
        updateApplyState();
        setCustomOpen(!customOpen);
        return;
      }
      // Preset (30d/7d): desaplica o range personalizado e restaura o rótulo.
      days = Number(d) || 30;
      customActive = false;
      customBtn.textContent = CUSTOM_LABEL;
      setOn(".codex-ranges", b);
      setCustomOpen(false);
      void loadCodexDashboard({ skeleton: true });
    }));

  const from = el("codex-range-from") as HTMLInputElement;
  const to = el("codex-range-to") as HTMLInputElement;
  from.oninput = updateApplyState;
  to.oninput = updateApplyState;

  (el("codex-range-apply") as HTMLButtonElement).onclick = () => {
    if (!(from.value && to.value)) return;
    customFrom = from.value;
    customTo = to.value;
    customActive = true;
    const { from: f, to: t } = customRange();
    customBtn.textContent = fmtShort(f) + " – " + fmtShort(t);
    setOn(".codex-ranges", customBtn);
    setCustomOpen(false);
    void loadCodexDashboard({ skeleton: true });
  };

  // Fecha o popover ao clicar fora (exceto no botão "Personalizado") ou com Esc.
  document.addEventListener("mousedown", (e) => {
    if (!customOpen) return;
    const t = e.target as Node;
    const pop = el("codex-range-custom");
    if (pop.contains(t) || customBtn.contains(t)) return;
    setCustomOpen(false);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && customOpen) setCustomOpen(false);
  });

  void loadCodexDashboard();
}
