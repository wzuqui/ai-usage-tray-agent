// Widget flutuante da área de trabalho (janela `widget`, sem moldura, sempre na
// frente). Mostra um card compacto por provedor (Claude/Codex) conforme as
// preferências da aba "Widget" das Configurações. Os dados vêm do comando
// `get_widget_state` (mesmo snapshot do tray/“Uso atual”, sem rede). O fundo
// (imagem/gif) é lido sob demanda via `read_widget_background` e aplicado como
// background do painel; a opacidade controla o quanto o fundo aparece.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  barColor,
  escapeHtml,
  fmtRemaining,
  fmtResetClock,
  ICON_CLAUDE,
  iconCodex,
  pctText,
  type ProviderUsage,
} from "./usage-format";

const WIDGET_WIDTH = 320;

interface WidgetState {
  habilitado: boolean;
  mostraClaude: boolean;
  mostraCodex: boolean;
  fundo: string;
  opacidade: number;
  janelas: string;
  formatoReset: string;
  sempreNaFrente: boolean;
  paused: boolean;
  claude: ProviderUsage;
  codex: ProviderUsage;
}

/// "sessao" → só 5h; "semanal" → só 7d; resto (inclusive "ambos") → as duas.
function parseJanelas(value: string): { sessao: boolean; semanal: boolean } {
  const v = (value ?? "").trim().toLowerCase();
  // Aceita os mesmos sinônimos do backend (parse_janelas em lib.rs).
  if (v === "sessao" || v === "sessão" || v === "session" || v === "5h") return { sessao: true, semanal: false };
  if (v === "semanal" || v === "semana" || v === "weekly" || v === "7d") return { sessao: false, semanal: true };
  return { sessao: true, semanal: true };
}

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;

let lastFundo: string | null = null;

/// Bloco compacto de uma janela (sessão 5h ou semanal 7d): rótulo curto + % +
/// barra fina + reset. Com `exact`, mostra a hora/data exata do reset (estática);
/// senão, o tempo restante (que conta ao vivo). Omitido quando não há dados.
function windowBlock(
  label: string,
  pct: number | undefined | null,
  resetIso: string | null | undefined,
  exact: boolean,
): string {
  if (pct === undefined || pct === null) return "";
  const width = Math.max(0, Math.min(100, pct));
  let reset = "";
  if (resetIso) {
    reset = exact
      ? `<div class="wwin-reset">reset ${fmtResetClock(resetIso)}</div>`
      : `<div class="wwin-reset">reset em <span class="w-remain" data-reset="${escapeHtml(resetIso)}">${fmtRemaining(resetIso)}</span></div>`;
  }
  return `<div class="wwin">
    <div class="wwin-top"><span class="wwin-label">${label}</span><span class="wwin-pct">${pctText(pct)}%</span></div>
    <div class="wbar"><div class="wbar-fill" style="width:${width}%;background:${barColor(pct)}"></div></div>
    ${reset}
  </div>`;
}

/// Card compacto de um provedor. `null` quando não deve aparecer (provedor
/// desabilitado ou escondido do widget). `janelas` escolhe quais janelas
/// (sessão 5h / semanal 7d) renderizar.
function renderProvider(
  label: string,
  prov: ProviderUsage,
  mostra: boolean,
  janelas: { sessao: boolean; semanal: boolean },
  exact: boolean,
): string | null {
  if (!mostra || !prov.habilitado) return null;
  const icon = label === "Codex" ? iconCodex() : ICON_CLAUDE;
  const head = `<div class="wprov-head">${icon}<span class="wprov-name">${label}</span></div>`;

  const m = prov.metric;
  if (!m) {
    return `<div class="wprov">${head}<div class="wprov-note">Coletando…</div></div>`;
  }
  if (m.status === "erro" || m.erro) {
    return `<div class="wprov error">${head}<div class="wprov-note err">erro na coleta</div></div>`;
  }
  const blocks = [
    janelas.sessao ? windowBlock("Sessão 5h", m.uso_percentual, m.reset_em, exact) : "",
    janelas.semanal ? windowBlock("Semanal 7d", m.uso_percentual_7d, m.reset_em_7d, exact) : "",
  ].join("");
  return `<div class="wprov">${head}
    <div class="wwins">${blocks}</div>
  </div>`;
}

/// Atualiza só a contagem regressiva, sem reconstruir os cards (a cada 1s).
function tick(): void {
  document.querySelectorAll<HTMLElement>(".w-remain[data-reset]").forEach((n) => {
    n.textContent = fmtRemaining(n.dataset.reset as string);
  });
}

// Auto-ajuste da altura ao conteúdo **só até o usuário redimensionar**. Depois
// que o usuário escolhe um tamanho (marcado em localStorage), respeitamos e o
// window-state cuida de salvar/restaurar; o auto-ajuste não age mais.
const PADDING = 24; // padding vertical do .wdg (12px topo + 12px base)
let userSized = localStorage.getItem("wdg-sized") === "1";
let selfResizing = false;
let lastHeight = 0;

const win = getCurrentWindow();
// Marca como "dimensionado pelo usuário" quando o resize não foi nosso.
void win.onResized(() => {
  if (selfResizing) return;
  userSized = true;
  localStorage.setItem("wdg-sized", "1");
});

/// Ajusta a altura da janela ao conteúdo (mede o .wdg-cards, não o .wdg, que é
/// clampado por min-height). Largura mantém o padrão. No-op após o usuário
/// redimensionar.
function fitToContent(): void {
  if (userSized) return;
  const h = Math.ceil(el("wdg-cards").getBoundingClientRect().height) + PADDING;
  if (h <= 0 || Math.abs(h - lastHeight) < 2) return;
  lastHeight = h;
  selfResizing = true;
  void win
    .setSize(new LogicalSize(WIDGET_WIDTH, h))
    .catch(() => {})
    .finally(() => setTimeout(() => { selfResizing = false; }, 120));
}

/// Aplica a imagem/gif de fundo só quando o caminho muda (ler/codificar é caro
/// para gifs grandes). Caminho vazio remove o fundo.
async function applyBackground(fundo: string): Promise<void> {
  if (fundo === lastFundo) return;
  lastFundo = fundo;
  const wdg = el("wdg");
  if (!fundo) {
    wdg.style.backgroundImage = "";
    return;
  }
  try {
    const dataUrl = await invoke<string | null>("read_widget_background");
    // Imagem + overlay escuro numa única camada de background: assim o recorte
    // dos cantos (border-radius + overflow: hidden) acontece uma só vez e não
    // sobra a borda clara que aparecia quando a imagem e o overlay estavam em
    // camadas separadas. O alpha do overlay segue --wdg-alpha (atualiza sozinho
    // quando a opacidade muda, sem reaplicar a imagem).
    wdg.style.backgroundImage = dataUrl
      ? `linear-gradient(rgba(26, 25, 21, var(--wdg-alpha)), rgba(26, 25, 21, var(--wdg-alpha))), url("${dataUrl}")`
      : "";
  } catch {
    wdg.style.backgroundImage = "";
  }
}

function render(state: WidgetState): void {
  // Opacidade do painel: 0..100 → alpha do fundo escuro do card.
  const alpha = Math.max(0, Math.min(100, state.opacidade)) / 100;
  el("wdg").style.setProperty("--wdg-alpha", String(alpha));

  const janelas = parseJanelas(state.janelas);
  const exact = state.formatoReset === "exato";
  const cards = [
    renderProvider("Claude", state.claude, state.mostraClaude, janelas, exact),
    renderProvider("Codex", state.codex, state.mostraCodex, janelas, exact),
  ].filter((c): c is string => c !== null);

  el("wdg-cards").innerHTML = cards.length
    ? cards.join("")
    : `<div class="wprov-note">Nenhum provedor selecionado.<br>Ative nas Configurações → Widget.</div>`;

  void applyBackground(state.fundo ?? "");
  tick();
  // Espera o layout para medir a altura real e casar a janela ao conteúdo.
  requestAnimationFrame(fitToContent);
}

/// Busca o estado pelo IPC e re-renderiza. Barata (sem rede no backend).
async function load(): Promise<void> {
  try {
    const state = await invoke<WidgetState>("get_widget_state");
    render(state);
  } catch {
    // Janela pode estar fechando; ignora.
  }
}

// A cada 1s atualiza a contagem regressiva; a cada 2s rebusca o estado (mesmo
// esquema da tela "Uso atual"). get_widget_state é barato e sem rede.
let tickCount = 0;
setInterval(() => {
  tick();
  if (++tickCount % 2 === 0) void load();
}, 1000);

// Clique direito em qualquer ponto do widget abre o menu do app (mesmos itens
// do tray), em vez do menu de contexto padrão do WebView.
window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void invoke("show_app_menu");
});

void load();