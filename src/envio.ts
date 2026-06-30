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
import { ICON_CLAUDE, iconCodex, escapeHtml, pctText } from "./usage-format";

interface ProviderEnvio {
  habilitado: boolean;
  enviar: boolean;
}
// Payload (dados) enviado ao Loki, anexado às entradas de sucesso para que o
// histórico mostre exatamente o que foi enviado. Espelha o body montado no backend.
interface SendPayload {
  uso_percentual?: number;
  restante_percentual?: number;
  status?: string;
  reset_em?: string | null;
  uso_percentual_7d?: number | null;
  restante_percentual_7d?: number | null;
  reset_em_7d?: string | null;
}
interface SendLogEntry {
  timestamp: string;
  ferramenta: string;
  status: string; // "sucesso" | "falha"
  detalhe: string | null;
  payload?: SendPayload | null;
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
// Chaves do histórico já vistas, para piscar só as entradas NOVAS.
const seenLog = new Set<string>();
// O "piscar" só é armado durante o poll ao vivo (pollEnvio). As renderizações de
// carga (loadEnvio, ao entrar na aba / focar a janela) são baseline: sincronizam
// o seenLog SEM piscar — senão, ao voltar de outra aba, todas as entradas que
// chegaram enquanto a tela estava inativa piscariam de uma vez.
let flashArmed = false;

const el = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;
const isActive = (): boolean => !!el("view-envio")?.classList.contains("on");

/// Nenhum provedor com "Enviar ao Loki" ativado: mesmo sem pausa, nada chega ao
/// Loki. A tela sinaliza isso (badge, sub-texto e aviso no histórico).
const noneSending = (): boolean => !!DATA && !DATA.claude.enviar && !DATA.codex.enviar;

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

/// Resumo legível do payload enviado ao Loki, exibido sob a linha do envio. Mostra
/// uso da sessão (5h), semanal (7d) e o reset; o `title` traz o payload cru (JSON),
/// para inspecionar exatamente o que foi enviado. "" quando não há payload.
function renderPayload(p: SendPayload | null | undefined): string {
  if (!p) return "";
  // Cada janela mostra seu próprio reset (sessão = reset_em, semanal = reset_em_7d).
  const resetTxt = (iso: string | null | undefined): string =>
    iso ? ` (reset ${escapeHtml(fmtLogTime(iso))})` : "";
  const parts: string[] = [];
  if (typeof p.uso_percentual === "number")
    parts.push(`Sessão (5h): <strong>${pctText(p.uso_percentual)}%</strong>${resetTxt(p.reset_em)}`);
  if (typeof p.uso_percentual_7d === "number")
    parts.push(`Semanal (7d): <strong>${pctText(p.uso_percentual_7d)}%</strong>${resetTxt(p.reset_em_7d)}`);
  if (!parts.length) return "";
  const raw = escapeHtml(JSON.stringify(p));
  return `<div class="envio-log-payload" title="${raw}">${parts.join(" · ")}</div>`;
}

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
  const none = noneSending();
  // Prioridade: pausa (controle geral) > nenhum provedor enviando > ativo.
  let badge: string;
  let sub: string;
  if (paused) {
    badge = '<span class="envio-badge paused">⏸ Envio pausado</span>';
    sub = "Os dados continuam sendo coletados; nada é enviado ao Loki.";
  } else if (none) {
    badge = '<span class="envio-badge none">⚠ Nenhum provedor enviando</span>';
    sub = 'Nenhum provedor está com "Enviar ao Loki" ativado.';
  } else {
    badge = '<span class="envio-badge active"><span class="envio-live"></span>Envio ativo</span>';
    sub = `<span id="envio-next">${nextSendText()}</span>`;
  }
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
  // Aviso fixo no topo do histórico quando nenhum provedor está enviando.
  const notice = noneSending()
    ? '<div class="envio-none-note">⚠ Nenhum provedor habilitado para envio. Ative "Enviar ao Loki" no Claude ou no Codex em <strong>Configurações</strong> para que os envios voltem a acontecer.</div>'
    : "";
  if (!DATA.log.length) {
    el("envio-log").innerHTML = notice + '<div class="envio-empty">Nenhum envio registrado ainda.</div>';
    return;
  }
  const rows = DATA.log
    .map((e) => {
      const ok = e.status === "sucesso";
      const key = `${e.timestamp}|${e.ferramenta}|${e.status}`;
      // Pisca só entradas novas e só quando armado (poll ao vivo), nunca no baseline.
      const isNew = flashArmed && !seenLog.has(key);
      seenLog.add(key);
      // Linha de detalhe (abaixo do cabeçalho): dados enviados no sucesso, erro na falha.
      const detail = ok
        ? renderPayload(e.payload)
        : e.detalhe
          ? `<div class="envio-log-det">${escapeHtml(e.detalhe)}</div>`
          : "";
      const badge = `<span class="envio-log-badge ${ok ? "ok" : "err"}">${ok ? "enviado" : "falha"}</span>`;
      return `<div class="envio-log-row${ok ? "" : " err"}${isNew ? " envio-log-new" : ""}">
        <span class="envio-log-time" data-ts="${escapeHtml(e.timestamp)}">${fmtLogTime(e.timestamp)}</span>
        <div class="envio-log-content">
          <div class="envio-log-line">
            <span class="envio-log-tool">${iconFor(e.ferramenta)} ${nameFor(e.ferramenta)}</span>
            ${badge}
          </div>
          ${detail}
        </div>
      </div>`;
    })
    .join("");
  el("envio-log").innerHTML = notice + rows;
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
  // Baseline: não pisca nada nesta carga (entrar na aba / focar a janela). Desarma
  // antes do await para que um poll concorrente durante a busca também não pisque.
  flashArmed = false;
  try {
    DATA = await invoke<EnvioState>("get_envio_state");
  } catch (e) {
    el("envio-foot").textContent = "Falha ao carregar: " + (e instanceof Error ? e.message : String(e));
    return;
  }
  render();
  // A partir daqui, novas entradas vistas pelo poll ao vivo voltam a piscar.
  flashArmed = true;
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