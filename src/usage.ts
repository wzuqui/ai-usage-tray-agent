// Tela "Uso atual": mostra o uso de sessão (5h) e semanal (7d) do Claude e do
// Codex, com barra de progresso, tempo restante para o reset (contagem ao vivo)
// e a data/hora exata do reset. Os dados vêm do comando IPC `get_usage`, que lê
// o mesmo snapshot usado pelo tray e pela barra de tarefas (sem rede). O botão
// "Atualizar agora" chama `force_collect`, que força uma coleta nova.
import { invoke } from "@tauri-apps/api/core";

interface UsageMetric {
  ferramenta: string;
  uso_percentual: number;
  restante_percentual: number;
  status: string;
  coletado_em: string;
  reset_em: string | null;
  erro: string | null;
  uso_percentual_7d?: number;
  reset_em_7d?: string | null;
}
interface ProviderUsage {
  habilitado: boolean;
  metric: UsageMetric | null;
}
interface Usage {
  paused: boolean;
  lastError: string | null;
  claude: ProviderUsage;
  codex: ProviderUsage;
}

// Ícones dos provedores no cabeçalho de cada card. Claude usa o "spark" laranja;
// o Codex usa o logo do Codex CLI (blob com gradiente roxo→azul e ">_" brancos),
// reproduzido como SVG inline.
const ICON_CLAUDE = '<span class="spark">✳</span>';
const ICON_CODEX =
  '<svg class="pico" width="20" height="20" viewBox="0 0 100 100" aria-hidden="true">' +
    '<defs><linearGradient id="codexg" x1="22" y1="6" x2="80" y2="98" gradientUnits="userSpaceOnUse">' +
      '<stop offset="0" stop-color="#a48cf2"/><stop offset=".55" stop-color="#5b62ec"/>' +
      '<stop offset="1" stop-color="#3a3fe6"/></linearGradient></defs>' +
    '<g fill="url(#codexg)">' +
      '<circle cx="50" cy="50" r="30"/><circle cx="50" cy="24" r="15"/><circle cx="68" cy="32" r="15"/>' +
      '<circle cx="76" cy="50" r="15"/><circle cx="68" cy="68" r="15"/><circle cx="50" cy="76" r="15"/>' +
      '<circle cx="32" cy="68" r="15"/><circle cx="24" cy="50" r="15"/><circle cx="32" cy="32" r="15"/>' +
    '</g>' +
    '<path d="M40 36 L53 50 L40 64" fill="none" stroke="#fff" stroke-width="9" ' +
      'stroke-linecap="round" stroke-linejoin="round"/>' +
    '<rect x="55" y="57" width="19" height="8" rx="4" fill="#fff"/>' +
  '</svg>';

let DATA: Usage | null = null;
let initialized = false;
let tickCount = 0;

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;
const isActive = (): boolean => !!el("view-usage")?.classList.contains("on");

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] as string);
}

/// % sem casas quando inteiro, com 1 casa quando fracionário.
function pctText(n: number): string {
  return Number.isInteger(n) ? String(n) : n.toFixed(1);
}

/// Cor da barra conforme o nível de uso: verde < 50% < amarelo < 80% < vermelho.
function barColor(pct: number): string {
  if (pct >= 80) return "#e0816f";
  if (pct >= 50) return "#d9b35a";
  return "#7fc99a";
}

/// Tempo restante até o reset, humanizado: "5d 2h", "3h 33min", "12min", "40s".
function fmtRemaining(iso: string): string {
  const ms = new Date(iso).getTime() - Date.now();
  if (Number.isNaN(ms)) return "—";
  if (ms <= 0) return "resetando…";
  const totalMin = Math.floor(ms / 60000);
  const days = Math.floor(totalMin / 1440);
  const hours = Math.floor((totalMin % 1440) / 60);
  const mins = totalMin % 60;
  if (days >= 1) return `${days}d ${hours}h`;
  if (hours >= 1) return `${hours}h ${mins}min`;
  if (totalMin >= 1) return `${mins}min`;
  return `${Math.floor(ms / 1000)}s`;
}

/// Data/hora exata do reset, no fuso local: "qui., 19/06, 17:00".
function fmtExact(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleString("pt-BR", {
    weekday: "short", day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit",
  });
}

/// Só o horário do reset, no fuso local: "14:29".
function fmtTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}

/// Quão recente é o dado coletado: "atualizado agora", "há 12s", "há 3min"…
function fmtFresh(iso: string): string {
  const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (Number.isNaN(s)) return "";
  if (s < 2) return "atualizado agora";
  if (s < 60) return `atualizado há ${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `atualizado há ${m}min`;
  const h = Math.floor(m / 60);
  if (h < 24) return `atualizado há ${h}h`;
  return `atualizado há ${Math.floor(h / 24)}d`;
}

/// Bloco de uma janela (sessão ou semanal): % + barra + reset (tempo e data).
/// `timeOnly` (sessão 5h): mostra "Horário: 14:29" (hora em branco) no lugar da
/// data completa, já que o reset é no mesmo dia.
function windowBlock(
  label: string,
  pct: number | undefined,
  resetIso: string | null | undefined,
  timeOnly: boolean,
): string {
  if (pct === undefined || pct === null) {
    return `<div class="uwin">
      <div class="uwin-top"><span class="uwin-label">${label}</span><span class="uwin-pct muted">—</span></div>
      <div class="uwin-na">Sem dados desta janela.</div>
    </div>`;
  }
  const width = Math.max(0, Math.min(100, pct));
  let reset: string;
  if (resetIso) {
    const when = timeOnly
      ? `<div class="ur-line"><span class="ur-k">Horário:</span> <span class="u-time">${fmtTime(resetIso)}</span></div>`
      : `<div class="ur-exact">${fmtExact(resetIso)}</div>`;
    reset = `<div class="ur-line"><span class="ur-k">Reset em</span> <span class="u-remain" data-reset="${resetIso}">${fmtRemaining(resetIso)}</span></div>${when}`;
  } else {
    reset = `<div class="ur-line ur-k">Sem horário de reset.</div>`;
  }
  return `<div class="uwin">
    <div class="uwin-top"><span class="uwin-label">${label}</span><span class="uwin-pct">${pctText(pct)}%</span></div>
    <div class="ubar"><div class="ubar-fill" style="width:${width}%;background:${barColor(pct)}"></div></div>
    <div class="uwin-reset">${reset}</div>
  </div>`;
}

/// Card de um provider, cobrindo os estados: desabilitado, sem dado ainda, erro
/// de coleta, ou as duas janelas (sessão e semanal). O ícone do cabeçalho é o do
/// provedor (Claude = spark; Codex = logo do Codex).
function renderProvider(label: string, prov: ProviderUsage): string {
  const icon = label === "Codex" ? ICON_CODEX : ICON_CLAUDE;
  const head = (meta: string) =>
    `<div class="uprov-head"><div class="uprov-name">${icon} ${label}</div><div class="uprov-meta">${meta}</div></div>`;

  if (!prov.habilitado) {
    return `<div class="uprov disabled">${head('<span class="ubadge muted">desabilitado</span>')}
      <div class="uprov-note">Habilite ${label} nas Configurações para coletar o uso.</div></div>`;
  }
  const m = prov.metric;
  if (!m) {
    return `<div class="uprov">${head("")}<div class="uprov-note">Coletando dados…</div></div>`;
  }
  if (m.status === "erro" || m.erro) {
    return `<div class="uprov error">${head('<span class="ubadge err">erro</span>')}
      <div class="uprov-note err">${escapeHtml(m.erro ?? "Falha na coleta.")}</div></div>`;
  }
  const meta = `<span class="u-fresh" data-collected="${m.coletado_em}">${fmtFresh(m.coletado_em)}</span>`;
  return `<div class="uprov">${head(meta)}
    <div class="uwins">
      ${windowBlock("Sessão (5h)", m.uso_percentual, m.reset_em, true)}
      ${windowBlock("Semanal (7d)", m.uso_percentual_7d, m.reset_em_7d, false)}
    </div>
  </div>`;
}

/// Atualiza só os textos dependentes do tempo (contagem regressiva e frescor),
/// sem reconstruir os cards — chamado a cada segundo.
function tick(): void {
  el("view-usage").querySelectorAll<HTMLElement>(".u-remain[data-reset]").forEach((n) => {
    n.textContent = fmtRemaining(n.dataset.reset as string);
  });
  el("view-usage").querySelectorAll<HTMLElement>(".u-fresh[data-collected]").forEach((n) => {
    n.textContent = fmtFresh(n.dataset.collected as string);
  });
}

function render(): void {
  if (!DATA) return;
  el("usage-banner").innerHTML = DATA.paused
    ? '<div class="ubanner">⏸ Coleta automática pausada. "Atualizar agora" busca dados novos; retome a coleta pelo menu do tray.</div>'
    : "";
  el("usage-cards").innerHTML =
    renderProvider("Claude", DATA.claude) + renderProvider("Codex", DATA.codex);
  el("usage-foot").textContent = "";
  tick();
}

/// Busca o snapshot pelo IPC e re-renderiza. Barata (sem rede no backend).
export async function loadUsage(): Promise<void> {
  try {
    DATA = await invoke<Usage>("get_usage");
  } catch (e) {
    el("usage-foot").textContent = "Falha ao carregar uso: " + (e instanceof Error ? e.message : String(e));
    return;
  }
  render();
}

/// Força uma coleta nova (também envia ao Loki) e mostra o resultado.
async function refresh(): Promise<void> {
  const btn = el("usage-refresh") as HTMLButtonElement;
  const prev = btn.textContent ?? "Atualizar agora";
  btn.disabled = true;
  btn.textContent = "Atualizando…";
  try {
    DATA = await invoke<Usage>("force_collect");
    render();
  } catch (e) {
    el("usage-foot").textContent = "Falha ao atualizar: " + (e instanceof Error ? e.message : String(e));
  } finally {
    btn.disabled = false;
    btn.textContent = prev;
  }
}

/// Liga os eventos (uma vez), inicia o tick de 1s e dispara o primeiro load.
export function initUsage(): void {
  if (initialized) { void loadUsage(); return; }
  initialized = true;

  (el("usage-refresh") as HTMLButtonElement).onclick = () => void refresh();

  // A cada 1s atualiza a contagem regressiva e o frescor (do dado em cache); a
  // cada 2s rebusca o snapshot. O rebusque precisa ser mais frequente que o
  // intervalo de coleta (mín. 5s) para o "atualizado há" subir de forma limpa e
  // zerar a cada coleta real, em vez de saltar de forma errática. Só roda quando
  // a tela está ativa, para não trabalhar à toa. get_usage é barato (sem rede).
  setInterval(() => {
    if (!isActive()) return;
    tick();
    if (++tickCount % 2 === 0) void loadUsage();
  }, 1000);

  void loadUsage();
}