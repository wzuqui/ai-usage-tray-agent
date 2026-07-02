// Painel de configurações nativo (abas), renderizado na webview.
// Lê/grava o config.json + autostart pelos comandos IPC `get_settings` e
// `save_settings`. Antes era um formulário servido por HTTP no navegador.
import { invoke } from "@tauri-apps/api/core";

interface CodexConfig {
  habilitado: boolean;
  mostraNaTaskbarWindows: boolean;
  authJsonPath: string;
  authMode: string;
}
interface ClaudeConfig {
  habilitado: boolean;
  mostraNaTaskbarWindows: boolean;
  organizationId: string;
  cookie: string;
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
  $<HTMLSelectElement>("set-codexAuthMode").value = codex.authMode === "navegador" ? "navegador" : "arquivo";
  syncCodexAuthMode();
  $<HTMLInputElement>("set-codexTaskbar").checked = codex.mostraNaTaskbarWindows !== false;

  $<HTMLInputElement>("set-claudeHab").checked = claude.habilitado !== false;
  $<HTMLInputElement>("set-claudeOrg").value = claude.organizationId ?? "";
  $<HTMLInputElement>("set-claudeCookie").value = claude.cookie ?? "";
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
        authMode: $<HTMLSelectElement>("set-codexAuthMode").value === "navegador" ? "navegador" : "arquivo",
      },
      claude: {
        habilitado: $<HTMLInputElement>("set-claudeHab").checked,
        mostraNaTaskbarWindows: $<HTMLInputElement>("set-claudeTaskbar").checked,
        organizationId: $<HTMLInputElement>("set-claudeOrg").value.trim(),
        cookie: $<HTMLInputElement>("set-claudeCookie").value.trim(),
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

/// Abre o seletor de arquivo nativo para escolher o auth.json do Codex e joga o
/// caminho no campo. Como o campo é alterado por código (não dispara "input"),
/// atualiza os avisos e agenda o auto-save explicitamente.
async function pickCodexAuthFile(): Promise<void> {
  try {
    const path = await invoke<string | null>("pick_codex_auth_file");
    if (path) {
      $<HTMLInputElement>("set-codexAuth").value = path;
      syncProviderHints();
      scheduleAutoSave();
    }
  } catch (e) {
    setMsg("Falha ao escolher arquivo: " + (e instanceof Error ? e.message : String(e)), "err");
  }
}

function setMsg(text: string, kind?: "ok" | "err"): void {
  // Só exibimos erros no topo — coisas que o usuário talvez não perceba (ex.: falha
  // ao salvar). Confirmações de ações que ele mesmo disparou (conectar/desconectar)
  // são redundantes e limpam a área em vez de exibir.
  const node = document.getElementById("settings-msg");
  if (!node) return;
  if (kind === "err" && text) {
    node.textContent = text;
    node.className = "msg err";
  } else {
    node.textContent = "";
    node.className = "msg";
  }
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

// Autenticação do Codex: o modo ("arquivo" | "navegador") é parte do config.json
// (auto-save), mas o login/logout pelo navegador é feito à parte, via os comandos
// codex_login/codex_logout, e o status vem de codex_auth_status.
interface CodexAuthStatus {
  connected: boolean;
  needsReconnect: boolean;
  email: string | null;
  expiresAt: number | null;
}
// Reflete o último status conhecido; usado pelos avisos de "sem credenciais".
let codexConnected = false;

/// Mostra a seção do modo escolhido (caminho do auth.json ou login pelo navegador).
function codexAuthMode(): "arquivo" | "navegador" {
  return $<HTMLSelectElement>("set-codexAuthMode").value === "navegador" ? "navegador" : "arquivo";
}
function syncCodexAuthMode(): void {
  const mode = codexAuthMode();
  ($("set-codexFileAuth") as HTMLElement).hidden = mode !== "arquivo";
  ($("set-codexBrowserAuth") as HTMLElement).hidden = mode !== "navegador";
}

/// Reflete o status do login pelo navegador na UI (texto, botões) e nos avisos.
/// - Conectado e saudável: só status + Desconectar (sem botão de conectar).
/// - Conectado mas com falha de renovação automática: mostra "Reconectar".
/// - Não conectado: mostra "Conectar com o navegador".
function applyCodexAuthStatus(st: CodexAuthStatus): void {
  // Para os avisos de coleta, uma sessão que precisa reconectar conta como "sem
  // credenciais" (a coleta não vai funcionar até reconectar).
  codexConnected = !!st.connected && !st.needsReconnect;
  const statusEl = $("set-codexAuthStatus");
  const loginBtn = $("set-codexLogin") as HTMLElement;
  const logoutBtn = $("set-codexLogout") as HTMLElement;

  statusEl.classList.remove("ok", "warn");
  if (st.connected && !st.needsReconnect) {
    statusEl.textContent = st.email ? `Conectado como ${st.email}.` : "Conectado.";
    statusEl.classList.add("ok");
    loginBtn.hidden = true;
    logoutBtn.hidden = false;
  } else if (st.connected && st.needsReconnect) {
    statusEl.textContent = "Não foi possível renovar a sessão automaticamente. Reconecte.";
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

/// Lê o status do login pelo navegador (sem rede). Chamado ao abrir a tela.
async function loadCodexAuthStatus(): Promise<void> {
  try {
    applyCodexAuthStatus(await invoke<CodexAuthStatus>("codex_auth_status"));
  } catch {
    // transitório; mantém o estado atual
  }
}

/// Dispara o login pelo navegador (bloqueante no backend até o usuário concluir ou
/// cancelar). Enquanto aguarda, mostra o botão "Cancelar" (que libera a porta 1455
/// e faz o comando retornar sem esperar o timeout).
async function codexLogin(): Promise<void> {
  const btn = $("set-codexLogin") as HTMLElement;
  const cancelBtn = $("set-codexLoginCancel") as HTMLElement;
  // Enquanto aguarda, esconde o "Conectar" e mostra o "Cancelar" no lugar.
  btn.hidden = true;
  cancelBtn.hidden = false;
  $("set-codexAuthStatus").textContent = "Aguardando o login no navegador…";
  try {
    applyCodexAuthStatus(await invoke<CodexAuthStatus>("codex_login"));
    setMsg("Codex conectado.", "ok");
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (/cancel/i.test(msg)) {
      setMsg(""); // cancelamento é ação do usuário; não polui o topo com aviso
    } else {
      setMsg("Falha no login do Codex: " + msg, "err");
    }
    // Reavalia o estado (mostra "Conectar"/"Reconectar" conforme o caso).
    await loadCodexAuthStatus();
  } finally {
    // A visibilidade do "Conectar/Reconectar" é decidida por applyCodexAuthStatus;
    // aqui só escondemos o "Cancelar".
    cancelBtn.hidden = true;
  }
}

/// Cancela um login em andamento (o codexLogin pendente rejeita e se recupera).
async function codexLoginCancel(): Promise<void> {
  try {
    await invoke("codex_login_cancel");
  } catch {
    // best-effort; o login pendente ainda expira sozinho no timeout
  }
}

/// Remove as credenciais do login pelo navegador.
async function codexLogout(): Promise<void> {
  try {
    await invoke("codex_logout");
    applyCodexAuthStatus({ connected: false, needsReconnect: false, email: null, expiresAt: null });
    setMsg("Codex desconectado.", "ok");
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
    void loadCodexAuthStatus();
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

  const claudeOn = $<HTMLInputElement>("set-claudeHab").checked;
  setBodyEnabled("set-claudeBody", claudeOn);
  const claudeFalta =
    $<HTMLInputElement>("set-claudeOrg").value.trim() === "" ||
    $<HTMLInputElement>("set-claudeCookie").value.trim() === "";
  ($("set-claudeWarn") as HTMLElement).hidden = !(claudeOn && claudeFalta);

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
  const codexCfg = codexAuthMode() === "navegador"
    ? codexConnected
    : $<HTMLInputElement>("set-codexAuth").value.trim() !== "";
  const claudeOn = $<HTMLInputElement>("set-claudeHab").checked;
  const claudeCfg =
    $<HTMLInputElement>("set-claudeOrg").value.trim() !== "" &&
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
  $("set-codexAuthMode").addEventListener("change", () => {
    syncCodexAuthMode();
    syncProviderHints();
  });
  $("set-codexAuthPick").addEventListener("click", () => void pickCodexAuthFile());
  $("set-codexLogin").addEventListener("click", () => void codexLogin());
  $("set-codexLoginCancel").addEventListener("click", () => void codexLoginCancel());
  $("set-codexLogout").addEventListener("click", () => void codexLogout());
  $("set-claudeHab").addEventListener("change", syncProviderHints);
  $("set-claudeOrg").addEventListener("input", syncProviderHints);
  $("set-claudeCookie").addEventListener("input", syncProviderHints);
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

  void loadSettings();
}