// Tela "Envio de dados": controla o envio das métricas ao Loki sem afetar a
// coleta. Mostra o estado atual (ativo/pausado), permite pausar/retomar e
// "Enviar agora" (geral), ligar/desligar o envio por provedor e ver o histórico
// dos últimos envios (sucesso/falha), atualizado quase em tempo real.
//
// Os dados vêm do comando IPC `get_envio_state` (barato, sem rede); as ações usam
// `set_envio_paused`, `set_envio_provider`, `envio_send_now` e `clear_send_log`.
// O estado da pausa é sincronizado com o menu do tray (mesma fonte no backend).
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
let sending = false;

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;
const isActive = (): boolean => !!el("view-envio")?.classList.contains("on");

/// "há 12s", "há 3min"… desde um instante ISO; "—" se inválido.
function fmtAgo(iso: string | null): string {
  if (!iso) return "—";
  const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (Number.isNaN(s)) return "—";
  if (s < 2) return "agora";
  if (s < 60) return `há ${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `há ${m}min`;
  const h = Math.floor(m / 60);
  if (h < 24) return `há ${h}h`;
  return `há ${Math.floor(h / 24)}d`;
}

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

/// Cabeçalho com o estado atual e os botões de pausar/retomar e enviar agora.
function renderState(): void {
  if (!DATA) return;
  const paused = DATA.paused;
  const badge = paused
    ? '<span class="envio-badge paused">⏸ Envio pausado</span>'
    : '<span class="envio-badge active">▶ Envio ativo</span>';
  const cadence = paused
    ? "Os dados continuam sendo coletados; nada é enviado ao Loki."
    : `Enviando automaticamente a cada ${DATA.intervaloSegundos}s.`;
  const last = `Último envio com sucesso: <strong>${fmtAgo(DATA.lastSuccessAt)}</strong>`;

  el("envio-state").innerHTML = `
    <div class="envio-card">
      <div class="envio-card-top">
        <div>${badge}<div class="envio-sub">${cadence}</div></div>
        <div class="envio-actions">
          <button type="button" class="btn" id="envio-toggle">${paused ? "Retomar envio" : "Pausar envio"}</button>
          <button type="button" class="btn btn-primary" id="envio-send"${sending ? " disabled" : ""}>${sending ? "Enviando…" : "Enviar agora"}</button>
        </div>
      </div>
      <div class="envio-meta">${last}</div>
    </div>`;

  (el("envio-toggle") as HTMLButtonElement).onclick = () => void togglePause();
  (el("envio-send") as HTMLButtonElement).onclick = () => void sendNow();
}

/// Banner de aviso quando o Loki não está configurado (envio não acontece).
function renderBanner(): void {
  if (!DATA) return;
  el("envio-banner").innerHTML = DATA.lokiConfigurado
    ? ""
    : '<div class="ubanner">⚠ URL do Loki não configurada. Os envios não acontecem até preencher a URL em Configurações → Geral.</div>';
}

/// Linha de um provedor: ícone/nome, estado da coleta e o toggle de envio.
function providerRow(ferramenta: string, prov: ProviderEnvio): string {
  const nome = nameFor(ferramenta);
  const coleta = prov.habilitado
    ? '<span class="envio-tag ok">coletando</span>'
    : '<span class="envio-tag off">coleta desabilitada</span>';
  return `
    <div class="envio-prov">
      <div class="envio-prov-name">${iconFor(ferramenta)} <span>${nome}</span> ${coleta}</div>
      <label class="check envio-switch">
        <input type="checkbox" data-prov="${ferramenta}" ${prov.enviar ? "checked" : ""}>
        <span>Enviar ao Loki</span>
      </label>
    </div>`;
}

function renderProviders(): void {
  if (!DATA) return;
  el("envio-providers").innerHTML =
    providerRow("claude", DATA.claude) + providerRow("codex", DATA.codex);
  el("envio-providers")
    .querySelectorAll<HTMLInputElement>('input[data-prov]')
    .forEach((input) => {
      input.onchange = () => void setProvider(input.dataset.prov as string, input.checked);
    });
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
      const dot = ok ? '<span class="envio-dot ok"></span>' : '<span class="envio-dot err"></span>';
      const detalhe = e.detalhe ? `<span class="envio-log-det">${escapeHtml(e.detalhe)}</span>` : "";
      return `<div class="envio-log-row${ok ? "" : " err"}">
        ${dot}
        <span class="envio-log-time" data-ts="${escapeHtml(e.timestamp)}">${fmtLogTime(e.timestamp)}</span>
        <span class="envio-log-tool">${iconFor(e.ferramenta)} ${nameFor(e.ferramenta)}</span>
        <span class="envio-log-status">${ok ? "sucesso" : "falha"}</span>
        ${detalhe}
      </div>`;
    })
    .join("");
  el("envio-log").innerHTML = rows;
}

function render(): void {
  renderBanner();
  renderState();
  renderProviders();
  renderLog();
  el("envio-foot").textContent = "";
}

/// Atualiza só os textos relativos ao tempo (sem reconstruir tudo), a cada 1s.
function tick(): void {
  if (!DATA) return;
  const last = el("envio-state").querySelector<HTMLElement>(".envio-meta strong");
  if (last) last.textContent = fmtAgo(DATA.lastSuccessAt);
}

/// Aplica um novo estado recebido do backend após uma ação do usuário e
/// re-renderiza a tela inteira, inclusive os toggles de provedor.
function apply(state: EnvioState): void {
  DATA = state;
  render();
}

/// Atualização leve do poll (a cada 2s): rebusca o estado e re-renderiza só o que
/// muda sozinho — pausa (pode vir do tray), último envio e histórico. Os toggles
/// de "Enviar ao Loki" refletem `config.envio`, que só muda por esta tela, então
/// não são reconstruídos no poll (reconstruí-los a cada 2s só arriscaria perder um
/// clique/foco). Mudanças neles vêm por `apply()`/`loadEnvio()`.
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

async function setProvider(ferramenta: string, enviar: boolean): Promise<void> {
  try {
    const state = await invoke<EnvioState>("set_envio_provider", { ferramenta, enviar });
    apply(state);
  } catch (e) {
    el("envio-foot").textContent = "Falha ao salvar: " + (e instanceof Error ? e.message : String(e));
    void loadEnvio();
  }
}

async function sendNow(): Promise<void> {
  sending = true;
  renderState();
  try {
    const state = await invoke<EnvioState>("envio_send_now");
    apply(state);
  } catch (e) {
    el("envio-foot").textContent = "Falha ao enviar: " + (e instanceof Error ? e.message : String(e));
  } finally {
    sending = false;
    renderState();
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

  // A cada 1s atualiza o "há Xs"; a cada 2s rebusca o estado (histórico em quase
  // tempo real) via pollEnvio, que re-renderiza só banner/estado/log — sem
  // reconstruir os toggles de provedor. Só trabalha com a tela ativa.
  setInterval(() => {
    if (!isActive()) return;
    tick();
    if (++tickCount % 2 === 0 && !sending) void pollEnvio();
  }, 1000);

  void loadEnvio();
}