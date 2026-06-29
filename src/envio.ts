// Tela "Envio de dados": controla o envio das métricas ao Loki sem afetar a
// coleta. Mostra o estado atual (ativo/pausado), permite pausar/retomar e
// "Enviar agora" (geral) e ver o histórico dos últimos envios (sucesso/falha),
// atualizado quase em tempo real. O envio por provedor (Enviar ao Loki) ficou nas
// abas de cada provedor em Configurações (chama `set_envio_provider` direto).
//
// Os dados vêm do comando IPC `get_envio_state` (barato, sem rede); as ações usam
// `set_envio_paused`, `envio_send_now` e `clear_send_log`. O estado da pausa é
// sincronizado com o menu do tray (mesma fonte no backend).
import { invoke } from "@tauri-apps/api/core";
import { ICON_CLAUDE, iconCodex, escapeHtml } from "./usage-format";

interface ProviderEnvio {
  habilitado: boolean;
  enviar: boolean;
}
interface SendLogEntry {
  timestamp: string;
  ferramenta: string;
  status: string; // "sucesso" | "falha"
  detalhe: string | null;
}
interface EnvioState {
  paused: boolean;
  intervaloSegundos: number;
  lastSuccessAt: string | null;
  lokiConfigurado: boolean;
  claude: ProviderEnvio;
  codex: ProviderEnvio;
  log: SendLogEntry[];
}

let DATA: EnvioState | null = null;
let initialized = false;
let tickCount = 0;
// Chaves do histórico já vistas, para piscar só as entradas NOVAS (e nunca a
// lista inteira na primeira renderização).
const seenLog = new Set<string>();
let logInitialized = false;

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;
const isActive = (): boolean => !!el("view-envio")?.classList.contains("on");

/// Hora exata local de um envio: "14:29:05" se for hoje, ou "22/06 14:29" senão.
function fmtLogTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return sameDay
    ? d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit", second: "2-digit" })
    : d.toLocaleString("pt-BR", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
}

const iconFor = (ferramenta: string): string =>
  ferramenta === "codex" ? iconCodex() : ICON_CLAUDE;
const nameFor = (ferramenta: string): string =>
  ferramenta === "codex" ? "Codex" : "Claude";

/// Texto dinâmico do próximo envio automático: conta regressiva dentro do
/// intervalo, ancorada no último envio com sucesso (cicla via módulo, então
/// nunca trava). "" quando pausado.
function nextSendText(): string {
  if (!DATA || DATA.paused) return "";
  const interval = Math.max(1, DATA.intervaloSegundos);
  if (!DATA.lastSuccessAt) return "Aguardando primeiro envio…";
  const elapsed = (Date.now() - new Date(DATA.lastSuccessAt).getTime()) / 1000;
  if (Number.isNaN(elapsed)) return "";
  const mod = ((elapsed % interval) + interval) % interval;
  const sec = Math.max(1, Math.ceil(interval - mod));
  return `Próximo envio em ${sec}s`;
}

/// Cabeçalho simplificado: indicador "ao vivo" (ponto pulsante quando ativo, com
/// a contagem do próximo envio) e o botão de pausar/retomar, em uma única linha.
function renderState(): void {
  if (!DATA) return;
  const paused = DATA.paused;
  const badge = paused
    ? '<span class="envio-badge paused">⏸ Envio pausado</span>'
    : '<span class="envio-badge active"><span class="envio-live"></span>Envio ativo</span>';
  const sub = paused
    ? "Os dados continuam sendo coletados; nada é enviado ao Loki."
    : `<span id="envio-next">${nextSendText()}</span>`;
  const provState = (label: string, on: boolean): string =>
    `${label}: <span class="envio-prov-state ${on ? "on" : "off"}">${on ? "ativado" : "desativado"}</span>`;
  const provStatus = `${provState("Claude", DATA.claude.enviar)} · ${provState("Codex", DATA.codex.enviar)}`;

  el("envio-state").innerHTML = `
    <div class="envio-card">
      <div class="envio-card-top">
        <div>
          ${badge}
          <div class="envio-sub">${sub}</div>
          <div class="envio-prov-status">${provStatus}</div>
        </div>
        <div class="envio-actions">
          <button type="button" class="btn" id="envio-toggle">${paused ? "Retomar envio" : "Pausar envio"}</button>
        </div>
      </div>
    </div>`;

  (el("envio-toggle") as HTMLButtonElement).onclick = () => void togglePause();
}

/// Atualiza só a contagem do próximo envio (a cada 1s), sem reconstruir o card.
function tick(): void {
  const next = document.getElementById("envio-next");
  if (next) next.textContent = nextSendText();
}

/// Banner de aviso quando o Loki não está configurado (envio não acontece).
function renderBanner(): void {
  if (!DATA) return;
  el("envio-banner").innerHTML = DATA.lokiConfigurado
    ? ""
    : '<div class="ubanner">⚠ URL do Loki não configurada. Os envios não acontecem até preencher a URL em Configurações → Geral.</div>';
}

/// Histórico de envios (mais recentes no topo). Atualiza a cada rebusca.
function renderLog(): void {
  if (!DATA) return;
  if (!DATA.log.length) {
    el("envio-log").innerHTML = '<div class="envio-empty">Nenhum envio registrado ainda.</div>';
    return;
  }
  const rows = DATA.log
    .map((e) => {
      const ok = e.status === "sucesso";
      const key = `${e.timestamp}|${e.ferramenta}|${e.status}`;
      // Pisca só entradas novas (e nunca na 1ª renderização, p/ não piscar tudo).
      const isNew = logInitialized && !seenLog.has(key);
      seenLog.add(key);
      const dot = ok ? '<span class="envio-dot ok"></span>' : '<span class="envio-dot err"></span>';
      const detalhe = e.detalhe ? `<span class="envio-log-det">${escapeHtml(e.detalhe)}</span>` : "";
      return `<div class="envio-log-row${ok ? "" : " err"}${isNew ? " envio-log-new" : ""}">
        ${dot}
        <span class="envio-log-time" data-ts="${escapeHtml(e.timestamp)}">${fmtLogTime(e.timestamp)}</span>
        <span class="envio-log-tool">${iconFor(e.ferramenta)} ${nameFor(e.ferramenta)}</span>
        <span class="envio-log-status">${ok ? "sucesso" : "falha"}</span>
        ${detalhe}
      </div>`;
    })
    .join("");
  el("envio-log").innerHTML = rows;
  logInitialized = true;
}

function render(): void {
  renderBanner();
  renderState();
  renderLog();
  el("envio-foot").textContent = "";
}

/// Aplica um novo estado recebido do backend após uma ação do usuário e
/// re-renderiza a tela inteira.
function apply(state: EnvioState): void {
  DATA = state;
  render();
}

/// Atualização leve do poll (a cada 2s): rebusca o estado e re-renderiza só o que
/// muda sozinho — pausa (pode vir do tray), último envio e histórico.
async function pollEnvio(): Promise<void> {
  let state: EnvioState;
  try {
    state = await invoke<EnvioState>("get_envio_state");
  } catch {
    return; // transitório; mantém o que está na tela
  }
  DATA = state;
  renderBanner();
  renderState();
  renderLog();
}

export async function loadEnvio(): Promise<void> {
  try {
    DATA = await invoke<EnvioState>("get_envio_state");
  } catch (e) {
    el("envio-foot").textContent = "Falha ao carregar: " + (e instanceof Error ? e.message : String(e));
    return;
  }
  render();
}

async function togglePause(): Promise<void> {
  if (!DATA) return;
  try {
    const state = await invoke<EnvioState>("set_envio_paused", { paused: !DATA.paused });
    apply(state);
  } catch (e) {
    el("envio-foot").textContent = "Falha ao alterar a pausa: " + (e instanceof Error ? e.message : String(e));
  }
}

async function clearLog(): Promise<void> {
  try {
    const state = await invoke<EnvioState>("clear_send_log");
    apply(state);
  } catch (e) {
    el("envio-foot").textContent = "Falha ao limpar: " + (e instanceof Error ? e.message : String(e));
  }
}

/// Liga os eventos (uma vez), inicia o tick de 1s e dispara o primeiro load.
export function initEnvio(): void {
  if (initialized) { void loadEnvio(); return; }
  initialized = true;

  (el("envio-clear") as HTMLButtonElement).onclick = () => void clearLog();

  // A cada 1s atualiza a contagem do próximo envio (tick); a cada ~2s rebusca o
  // estado (histórico em quase tempo real) via pollEnvio. Só com a tela ativa.
  setInterval(() => {
    if (!isActive()) return;
    tick();
    if (++tickCount % 2 === 0) void pollEnvio();
  }, 1000);

  void loadEnvio();
}