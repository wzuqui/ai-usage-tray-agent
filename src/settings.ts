// Painel de configurações nativo (abas), renderizado na webview.
// Lê/grava o config.json + autostart pelos comandos IPC `get_settings` e
// `save_settings`. Antes era um formulário servido por HTTP no navegador.
import { invoke } from "@tauri-apps/api/core";

interface CodexConfig {
  habilitado: boolean;
  mostraNaTaskbarWindows: boolean;
  authJsonPath: string;
}
interface ClaudeConfig {
  habilitado: boolean;
  mostraNaTaskbarWindows: boolean;
  organizationId: string;
  cookie: string;
  authMode: string;
}
interface BarraConfig {
  lado: string;
  deslocamento: number;
  tamanhoFonte: number;
  corFonte: string;
  formatoReset: string;
  janelas: string;
}
interface WidgetConfig {
  habilitado: boolean;
  mostraClaude: boolean;
  mostraCodex: boolean;
  fundo: string;
  sempreNaFrente: boolean;
  opacidade: number;
  janelas: string;
  formatoReset: string;
  modo: string;
}
interface ServerConfig {
  habilitado: boolean;
  host: string;
  porta: number;
  pin: string;
}
interface AppConfig {
  usuario: string;
  intervaloSegundos: number;
  loki: { url: string };
  providers: { codex: CodexConfig; claude: ClaudeConfig };
  barraTarefas: BarraConfig;
  widget: WidgetConfig;
  servidor: ServerConfig;
}
interface SettingsData {
  autostart: boolean;
  os: string;
  autostartLabel: string;
  appVersion: string;
  config: AppConfig;
}
interface SaveSettings {
  config: AppConfig;
  autostart: boolean;
}

const $ = <T extends HTMLElement = HTMLElement>(id: string): T => document.getElementById(id) as T;

function fillForm(data: SettingsData): void {
  const c = data.config;
  const codex = c.providers.codex;
  const claude = c.providers.claude;
  const barra = c.barraTarefas;
  const widget = c.widget;
  const servidor = c.servidor;

  $<HTMLInputElement>("set-usuario").value = c.usuario ?? "";
  $<HTMLInputElement>("set-intervalo").value = String(c.intervaloSegundos ?? 10);
  $<HTMLInputElement>("set-lokiUrl").value = c.loki?.url ?? "";

  $<HTMLInputElement>("set-codexHab").checked = codex.habilitado !== false;
  $<HTMLInputElement>("set-codexAuth").value = codex.authJsonPath ?? "";
  $<HTMLInputElement>("set-codexTaskbar").checked = codex.mostraNaTaskbarWindows !== false;

  $<HTMLInputElement>("set-claudeHab").checked = claude.habilitado !== false;
  $<HTMLInputElement>("set-claudeOrg").value = claude.organizationId ?? "";
  $<HTMLInputElement>("set-claudeCookie").value = claude.cookie ?? "";
  $<HTMLSelectElement>("set-claudeAuthMode").value = claude.authMode === "navegador" ? "navegador" : "manual";
  syncClaudeAuthMode();
  $<HTMLInputElement>("set-claudeTaskbar").checked = claude.mostraNaTaskbarWindows !== false;

  $<HTMLSelectElement>("set-barraLado").value = barra.lado === "esquerda" ? "esquerda" : "direita";
  $<HTMLInputElement>("set-barraDesloc").value = String(barra.deslocamento ?? 0);
  $<HTMLInputElement>("set-barraFonte").value = String(barra.tamanhoFonte ?? 9);
  $<HTMLInputElement>("set-barraCor").value = barra.corFonte ?? "auto";
  $<HTMLSelectElement>("set-barraFormatoReset").value = barra.formatoReset === "exato" ? "exato" : "restante";
  $<HTMLSelectElement>("set-barraJanelas").value = normJanelas(barra.janelas);
  syncColorPicker();

  $<HTMLInputElement>("set-wdgClaude").checked = widget?.mostraClaude !== false;
  $<HTMLInputElement>("set-wdgCodex").checked = widget?.mostraCodex !== false;
  $<HTMLInputElement>("set-wdgTopo").checked = widget?.sempreNaFrente !== false;
  $<HTMLInputElement>("set-wdgFundo").value = widget?.fundo ?? "";
  $<HTMLSelectElement>("set-wdgJanelas").value = normJanelas(widget?.janelas);
  $<HTMLSelectElement>("set-wdgFormatoReset").value = normResetMode(widget?.formatoReset);
  setWdgModo(widget?.modo);
  $<HTMLInputElement>("set-wdgOpac").value = String(widget?.opacidade ?? 90);
  syncOpacLabel();

  $<HTMLInputElement>("set-srvHab").checked = !!servidor?.habilitado;
  $<HTMLSelectElement>("set-srvHost").value = servidor?.host === "0.0.0.0" ? "0.0.0.0" : "127.0.0.1";
  $<HTMLInputElement>("set-srvPorta").value = String(servidor?.porta ?? 8770);
  $<HTMLInputElement>("set-srvPin").value = servidor?.pin ?? "";
  syncServerPinHint();
  syncProviderHints();

  $<HTMLInputElement>("set-autostart").checked = !!data.autostart;
  if (data.autostartLabel) $("set-autostartLabel").textContent = data.autostartLabel;
}

function collect(): SaveSettings {
  let intervalo = parseInt($<HTMLInputElement>("set-intervalo").value, 10);
  if (!Number.isFinite(intervalo)) intervalo = 10;
  let deslocamento = parseInt($<HTMLInputElement>("set-barraDesloc").value, 10);
  if (!Number.isFinite(deslocamento)) deslocamento = 0;
  let fonte = parseInt($<HTMLInputElement>("set-barraFonte").value, 10);
  if (!Number.isFinite(fonte)) fonte = 9;
  let opacidade = parseInt($<HTMLInputElement>("set-wdgOpac").value, 10);
  if (!Number.isFinite(opacidade)) opacidade = 90;
  let srvPorta = parseInt($<HTMLInputElement>("set-srvPorta").value, 10);
  if (!Number.isFinite(srvPorta) || srvPorta < 1 || srvPorta > 65535) srvPorta = 8770;

  const config: AppConfig = {
    usuario: $<HTMLInputElement>("set-usuario").value.trim(),
    intervaloSegundos: intervalo,
    loki: { url: $<HTMLInputElement>("set-lokiUrl").value.trim() },
    providers: {
      codex: {
        habilitado: $<HTMLInputElement>("set-codexHab").checked,
        mostraNaTaskbarWindows: $<HTMLInputElement>("set-codexTaskbar").checked,
        authJsonPath: $<HTMLInputElement>("set-codexAuth").value.trim(),
      },
      claude: {
        habilitado: $<HTMLInputElement>("set-claudeHab").checked,
        mostraNaTaskbarWindows: $<HTMLInputElement>("set-claudeTaskbar").checked,
        organizationId: $<HTMLInputElement>("set-claudeOrg").value.trim(),
        cookie: $<HTMLInputElement>("set-claudeCookie").value.trim(),
        authMode: $<HTMLSelectElement>("set-claudeAuthMode").value === "navegador" ? "navegador" : "manual",
      },
    },
    barraTarefas: {
      lado: $<HTMLSelectElement>("set-barraLado").value,
      deslocamento,
      tamanhoFonte: fonte,
      corFonte: $<HTMLInputElement>("set-barraCor").value.trim() || "auto",
      formatoReset: $<HTMLSelectElement>("set-barraFormatoReset").value,
      janelas: $<HTMLSelectElement>("set-barraJanelas").value,
    },
    widget: {
      // O widget aparece quando ao menos um provedor esta marcado; nao ha mais
      // um checkbox separado de "mostrar widget".
      habilitado: $<HTMLInputElement>("set-wdgClaude").checked || $<HTMLInputElement>("set-wdgCodex").checked,
      mostraClaude: $<HTMLInputElement>("set-wdgClaude").checked,
      mostraCodex: $<HTMLInputElement>("set-wdgCodex").checked,
      fundo: $<HTMLInputElement>("set-wdgFundo").value.trim(),
      sempreNaFrente: $<HTMLInputElement>("set-wdgTopo").checked,
      opacidade,
      janelas: $<HTMLSelectElement>("set-wdgJanelas").value,
      formatoReset: $<HTMLSelectElement>("set-wdgFormatoReset").value,
      modo: getWdgModo(),
    },
    servidor: {
      habilitado: $<HTMLInputElement>("set-srvHab").checked,
      host: $<HTMLSelectElement>("set-srvHost").value,
      porta: srvPorta,
      pin: $<HTMLInputElement>("set-srvPin").value.trim(),
    },
  };
  return { config, autostart: $<HTMLInputElement>("set-autostart").checked };
}

function syncColorPicker(): void {
  const v = $<HTMLInputElement>("set-barraCor").value.trim();
  if (/^#?[0-9a-fA-F]{6}$/.test(v)) {
    $<HTMLInputElement>("set-barraCorPicker").value = "#" + v.replace(/^#/, "");
  }
}

/// Reflete o valor do slider de opacidade no rótulo ao lado.
function syncOpacLabel(): void {
  $("set-wdgOpacVal").textContent = $<HTMLInputElement>("set-wdgOpac").value;
}

/// Normaliza a opção de janelas para um dos valores válidos do <select>.
function normJanelas(value: string | undefined): "ambos" | "sessao" | "semanal" {
  return value === "sessao" || value === "semanal" ? value : "ambos";
}

/// Normaliza o formato do reset para um valor do <select> (default "restante").
function normResetMode(value: string | undefined): "restante" | "exato" | "nenhum" {
  return value === "exato" || value === "nenhum" ? value : "restante";
}

/// Normaliza o modo de exibição do widget para um valor do <select>
/// (default "completo").
function normModo(value: string | undefined): "completo" | "minimo" | "anelduplo" {
  return value === "minimo" || value === "anelduplo" ? value : "completo";
}

/// Lê/grava o modo de exibição do widget pelo grupo de radios das miniaturas
/// (cada miniatura desenha o widget no respectivo modo e seleciona ao clicar).
function getWdgModo(): string {
  const checked = document.querySelector<HTMLInputElement>('input[name="wdgModo"]:checked');
  return normModo(checked?.value);
}
function setWdgModo(value: string | undefined): void {
  const mode = normModo(value);
  document.querySelectorAll<HTMLInputElement>('input[name="wdgModo"]').forEach((radio) => {
    radio.checked = radio.value === mode;
  });
}

/// Abre o seletor de arquivo nativo (no backend) e joga o caminho escolhido no
/// campo de fundo. Como o campo é alterado por código (não dispara "change"),
/// agenda o auto-save explicitamente.
async function pickBackground(): Promise<void> {
  try {
    const path = await invoke<string | null>("pick_widget_background");
    if (path) {
      $<HTMLInputElement>("set-wdgFundo").value = path;
      scheduleAutoSave();
    }
  } catch (e) {
    setMsg("Falha ao escolher arquivo: " + (e instanceof Error ? e.message : String(e)), "err");
  }
}

function setMsg(text: string, kind?: "ok" | "err"): void {
  const node = $("settings-msg");
  node.textContent = text;
  node.className = "msg" + (kind ? " " + kind : "");
}

// O bloco `envio` (Enviar ao Loki por provedor) NÃO faz parte do save_settings
// (o backend o preserva relendo do disco). Os toggles nas abas Codex/Claude leem
// de `get_envio_state` e gravam via `set_envio_provider`, à parte do auto-save.
interface EnvioToggles {
  claude: { enviar: boolean };
  codex: { enviar: boolean };
}

/// Reflete nos checkboxes "Enviar ao Loki" o estado atual de envio por provedor.
async function loadEnvioToggles(): Promise<void> {
  try {
    const st = await invoke<EnvioToggles>("get_envio_state");
    $<HTMLInputElement>("set-codexEnviar").checked = !!st.codex?.enviar;
    $<HTMLInputElement>("set-claudeEnviar").checked = !!st.claude?.enviar;
  } catch {
    // transitório; mantém o estado atual dos checkboxes
  }
}

/// Persiste o envio de um provedor direto via set_envio_provider.
async function setEnvioProvider(ferramenta: "codex" | "claude", enviar: boolean): Promise<void> {
  try {
    await invoke("set_envio_provider", { ferramenta, enviar });
  } catch (e) {
    setMsg("Falha ao salvar o envio do provedor: " + (e instanceof Error ? e.message : String(e)), "err");
  }
}

// Autenticação do Claude: o modo ("manual" | "navegador") é parte do config.json
// (auto-save); o login/logout pelo navegador é feito à parte, pelos comandos
// claude_login/claude_logout, e o status vem de claude_auth_status.
interface ClaudeAuthStatus {
  connected: boolean;
  needsReconnect: boolean;
  email: string | null;
  organizationId: string | null;
}
// Último status conhecido do login pelo navegador; usado nos avisos "sem credenciais".
let claudeConnected = false;
// Enquanto true, o auto-refresh não relê o status (não atropela o "Aguardando…").
let claudeLoginInProgress = false;

function claudeAuthMode(): "manual" | "navegador" {
  return $<HTMLSelectElement>("set-claudeAuthMode").value === "navegador" ? "navegador" : "manual";
}
/// Mostra a seção do modo escolhido (campos manuais ou login pelo navegador).
function syncClaudeAuthMode(): void {
  const mode = claudeAuthMode();
  ($("set-claudeManualAuth") as HTMLElement).hidden = mode !== "manual";
  ($("set-claudeBrowserAuth") as HTMLElement).hidden = mode !== "navegador";
}

/// Reflete o status do login pelo navegador na UI (texto, botões) e nos avisos.
function applyClaudeAuthStatus(st: ClaudeAuthStatus): void {
  // Sessão que precisa reconectar conta como "sem credenciais" nos avisos.
  claudeConnected = !!st.connected && !st.needsReconnect;
  const statusEl = $("set-claudeAuthStatus");
  const loginBtn = $("set-claudeLogin") as HTMLElement;
  const logoutBtn = $("set-claudeLogout") as HTMLElement;
  statusEl.classList.remove("ok", "warn");
  if (st.connected && !st.needsReconnect) {
    statusEl.textContent = st.email ? `Conectado como ${st.email}.` : "Conectado.";
    statusEl.classList.add("ok");
    loginBtn.hidden = true;
    logoutBtn.hidden = false;
  } else if (st.connected && st.needsReconnect) {
    statusEl.textContent = "Sessão expirada. Reconecte sua conta para continuar a coleta.";
    statusEl.classList.add("warn");
    loginBtn.hidden = false;
    loginBtn.textContent = "Reconectar";
    logoutBtn.hidden = false;
  } else {
    statusEl.textContent = "Conecte sua conta para iniciar a coleta.";
    loginBtn.hidden = false;
    loginBtn.textContent = "Conectar com o navegador";
    logoutBtn.hidden = true;
  }
  syncProviderHints();
}

async function loadClaudeAuthStatus(): Promise<void> {
  try {
    applyClaudeAuthStatus(await invoke<ClaudeAuthStatus>("claude_auth_status"));
  } catch {
    // transitório; mantém o estado atual
  }
}

/// Dispara o login pelo navegador (abre a claude.ai; o backend captura o cookie).
/// Enquanto aguarda, esconde "Conectar" e mostra "Cancelar".
async function claudeLogin(): Promise<void> {
  const btn = $("set-claudeLogin") as HTMLElement;
  const cancelBtn = $("set-claudeLoginCancel") as HTMLElement;
  btn.hidden = true;
  cancelBtn.hidden = false;
  claudeLoginInProgress = true;
  $("set-claudeAuthStatus").textContent = "Aguardando o login no navegador…";
  try {
    applyClaudeAuthStatus(await invoke<ClaudeAuthStatus>("claude_login"));
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (!/cancel/i.test(msg)) setMsg("Falha no login do Claude: " + msg, "err");
    await loadClaudeAuthStatus();
  } finally {
    claudeLoginInProgress = false;
    cancelBtn.hidden = true;
  }
}

async function claudeLoginCancel(): Promise<void> {
  try {
    await invoke("claude_login_cancel");
  } catch {
    // best-effort; o login pendente ainda expira sozinho no timeout
  }
}

async function claudeLogout(): Promise<void> {
  try {
    await invoke("claude_logout");
    applyClaudeAuthStatus({ connected: false, needsReconnect: false, email: null, organizationId: null });
  } catch (e) {
    setMsg("Falha ao desconectar: " + (e instanceof Error ? e.message : String(e)), "err");
  }
}

export async function loadSettings(): Promise<void> {
  setMsg("");
  try {
    const data = await invoke<SettingsData>("get_settings");
    fillForm(data);
    void loadEnvioToggles();
    void loadClaudeAuthStatus();
    $("settings-loading").hidden = true;
    $("settings-form").hidden = false;
  } catch (e) {
    $("settings-loading").textContent = "Falha ao carregar configurações: " + (e instanceof Error ? e.message : String(e));
  }
}

let saveTimer: number | undefined;
// Cresce a cada agendamento; a resposta de um save só re-preenche o formulário
// se nenhuma mudança nova ocorreu nesse meio-tempo (evita sobrescrever o que o
// usuário acabou de alterar enquanto o save anterior estava em voo).
let saveSeq = 0;

/// Há um campo de texto/número em foco? Nesse caso o auto-save não deve
/// re-preencher o formulário (sobrescreveria o que está sendo digitado). Para
/// checkbox/select/range re-preencher é inofensivo (o valor já bate).
function isEditingField(): boolean {
  const a = document.activeElement as HTMLInputElement | null;
  if (!a || a.tagName !== "INPUT") return false;
  return a.type === "text" || a.type === "number" || a.type === "password";
}

/// Agenda um save com debounce: alterações em rajada (vários toggles, digitação)
/// são unificadas num único envio. Substitui o antigo botão "Salvar".
function scheduleAutoSave(): void {
  saveSeq++;
  if (saveTimer !== undefined) clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveTimer = undefined;
    void autoSave();
  }, 400);
}

async function autoSave(): Promise<void> {
  const seq = saveSeq;
  try {
    const data = await invoke<SettingsData>("save_settings", { settings: collect() });
    // Só reflete a normalização (clamp de intervalo/fonte, validação de cor) se
    // não houve mudança nova e nada está sendo digitado.
    if (seq === saveSeq && !isEditingField()) fillForm(data);
    setMsg("");
  } catch (e) {
    setMsg("Erro ao salvar: " + (e instanceof Error ? e.message : String(e)), "err");
  }
}

/// Mostra o aviso de PIN obrigatório quando o servidor está habilitado mas sem
/// PIN — deixa claro que, sem PIN, ele não inicia.
function syncServerPinHint(): void {
  const habilitado = $<HTMLInputElement>("set-srvHab").checked;
  setBodyEnabled("set-srvBody", habilitado);
  const semPin = $<HTMLInputElement>("set-srvPin").value.trim() === "";
  ($("set-srvPinWarn") as HTMLElement).hidden = !(habilitado && semPin);
}

/// Habilita/desabilita (e esmaece) o corpo de campos de um provedor conforme ele
/// esteja ligado. Os valores continuam legíveis para o save; só não são editáveis.
function setBodyEnabled(bodyId: string, on: boolean): void {
  const body = $(bodyId);
  body.classList.toggle("is-off", !on);
  body.querySelectorAll("input, button").forEach((el) => {
    (el as HTMLInputElement | HTMLButtonElement).disabled = !on;
  });
}

/// Reflete o estado de cada provedor: desabilita os campos quando desligado e, se
/// ligado, avisa que faltam os campos obrigatórios para a coleta acontecer
/// (Codex: auth.json; Claude: organization id + cookie).
function syncProviderHints(): void {
  const codexOn = $<HTMLInputElement>("set-codexHab").checked;
  setBodyEnabled("set-codexBody", codexOn);
  const codexFalta = $<HTMLInputElement>("set-codexAuth").value.trim() === "";
  ($("set-codexWarn") as HTMLElement).hidden = !(codexOn && codexFalta);

  const claudeOn = $<HTMLInputElement>("set-claudeHab").checked;
  setBodyEnabled("set-claudeBody", claudeOn);
  // No login pelo navegador o estado já aparece no status do bloco; o aviso amarelo
  // só é usado no modo manual (org + cookie).
  if (claudeAuthMode() === "navegador") {
    ($("set-claudeWarn") as HTMLElement).hidden = true;
  } else {
    const claudeFalta =
      $<HTMLInputElement>("set-claudeOrg").value.trim() === "" ||
      $<HTMLInputElement>("set-claudeCookie").value.trim() === "";
    ($("set-claudeWarn") as HTMLElement).hidden = !(claudeOn && claudeFalta);
  }

  syncProviderNotes();
}

/// Aviso por provedor (abas Envio, Barra de tarefas e Widget): se o provedor está
/// desativado ou sem credenciais, avisa — mas o toggle segue operável. `suffix`
/// completa o texto (Envio explica "não há dados para enviar"; barra/widget não).
function providerNote(habilitado: boolean, configurado: boolean, suffix: string): string {
  if (!habilitado) return `Provedor desativado${suffix}`;
  if (!configurado) return `Sem credenciais${suffix}`;
  return "";
}
function setNotes(ids: string[], msg: string): void {
  ids.forEach((id) => {
    const note = document.getElementById(id);
    if (!note) return;
    note.textContent = msg;
    (note as HTMLElement).hidden = msg === "";
  });
}
function syncProviderNotes(): void {
  const codexOn = $<HTMLInputElement>("set-codexHab").checked;
  const codexCfg = $<HTMLInputElement>("set-codexAuth").value.trim() !== "";
  const claudeOn = $<HTMLInputElement>("set-claudeHab").checked;
  const claudeCfg = claudeAuthMode() === "navegador"
    ? claudeConnected
    : $<HTMLInputElement>("set-claudeOrg").value.trim() !== "" &&
      $<HTMLInputElement>("set-claudeCookie").value.trim() !== "";
  setNotes(["envio-codex-note"], providerNote(codexOn, codexCfg, " — não há dados para enviar."));
  setNotes(["envio-claude-note"], providerNote(claudeOn, claudeCfg, " — não há dados para enviar."));
  setNotes(["barra-codex-note", "wdg-codex-note"], providerNote(codexOn, codexCfg, ""));
  setNotes(["barra-claude-note", "wdg-claude-note"], providerNote(claudeOn, claudeCfg, ""));
}

function activateTab(tab: string): void {
  document.querySelectorAll(".settings-tabs button").forEach((b) =>
    b.classList.toggle("on", (b as HTMLElement).dataset.stab === tab));
  document.querySelectorAll(".stab").forEach((s) =>
    s.classList.toggle("on", (s as HTMLElement).dataset.spanel === tab));
}

let initialized = false;

/// Liga os eventos da seção (uma vez) e carrega os valores atuais. Chamada na
/// primeira vez que o usuário abre a aba Configurações.
export function initSettings(): void {
  if (initialized) { void loadSettings(); return; }
  initialized = true;

  document.querySelectorAll(".settings-tabs button").forEach((b) =>
    b.addEventListener("click", () => activateTab((b as HTMLElement).dataset.stab ?? "geral")));

  $("set-cookieToggle").addEventListener("click", () => {
    const input = $<HTMLInputElement>("set-claudeCookie");
    const show = input.type === "password";
    input.type = show ? "text" : "password";
    $("set-cookieToggle").textContent = show ? "Ocultar" : "Mostrar";
  });
  $("set-srvPinToggle").addEventListener("click", () => {
    const input = $<HTMLInputElement>("set-srvPin");
    const show = input.type === "password";
    input.type = show ? "text" : "password";
    $("set-srvPinToggle").textContent = show ? "Ocultar" : "Mostrar";
  });
  $("set-srvHab").addEventListener("change", syncServerPinHint);
  $("set-srvPin").addEventListener("input", syncServerPinHint);
  $("set-codexHab").addEventListener("change", syncProviderHints);
  $("set-codexAuth").addEventListener("input", syncProviderHints);
  $("set-claudeHab").addEventListener("change", syncProviderHints);
  $("set-claudeOrg").addEventListener("input", syncProviderHints);
  $("set-claudeCookie").addEventListener("input", syncProviderHints);
  $("set-claudeAuthMode").addEventListener("change", () => {
    syncClaudeAuthMode();
    syncProviderHints();
  });
  $("set-claudeLogin").addEventListener("click", () => void claudeLogin());
  $("set-claudeLoginCancel").addEventListener("click", () => void claudeLoginCancel());
  $("set-claudeLogout").addEventListener("click", () => void claudeLogout());
  $("set-barraCor").addEventListener("input", syncColorPicker);
  $("set-barraCorPicker").addEventListener("input", () => {
    $<HTMLInputElement>("set-barraCor").value = $<HTMLInputElement>("set-barraCorPicker").value;
  });

  $("set-wdgOpac").addEventListener("input", syncOpacLabel);
  $("set-wdgFundoPick").addEventListener("click", () => void pickBackground());
  $("set-wdgFundoClear").addEventListener("click", () => {
    $<HTMLInputElement>("set-wdgFundo").value = "";
    scheduleAutoSave();
  });

  // "Enviar ao Loki" por provedor: persistido via set_envio_provider, à parte do
  // auto-save (stopPropagation evita o save_settings geral, que apenas preservaria
  // o bloco `envio` de qualquer forma).
  const wireEnvio = (id: string, ferramenta: "codex" | "claude"): void => {
    $(id).addEventListener("change", (e) => {
      e.stopPropagation();
      void setEnvioProvider(ferramenta, $<HTMLInputElement>(id).checked);
    });
  };
  wireEnvio("set-codexEnviar", "codex");
  wireEnvio("set-claudeEnviar", "claude");

  // Auto-save: qualquer alteração nos controles (toggle, select, slider, ou ao
  // sair de um campo de texto) persiste sozinha. O evento "change" borbulha, então
  // um único listener no formulário cobre todos os campos. Setar valores por
  // código (fillForm, picker de cor/fundo) não dispara "change", logo não há laço.
  $("settings-form").addEventListener("change", () => scheduleAutoSave());

  // Auto-refresh do status do login pelo navegador: enquanto a tela Configurações
  // está visível, relê o status a cada 5s para refletir mudanças externas (ex.:
  // sessão expirou e a coleta marcou "Reconectar"). Só relê o status (texto/botões),
  // sem tocar nos campos; pula durante um login em andamento. A janela é destruída
  // ao fechar, então o timer não vaza entre reaberturas.
  window.setInterval(() => {
    const visivel = document.getElementById("view-settings")?.classList.contains("on");
    if (!visivel || claudeLoginInProgress) return;
    if (claudeAuthMode() === "navegador") void loadClaudeAuthStatus();
    // Quando o login pelo navegador do Codex entrar (PR #39), refrescar aqui também:
    // if (codexAuthMode() === "navegador") void loadCodexAuthStatus();
  }, 5000);

  void loadSettings();
}