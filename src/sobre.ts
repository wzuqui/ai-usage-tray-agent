// Tela "Sobre" (última opção do menu): versão instalada, verificação de
// atualização (inline, sem diálogo) e o histórico de Novidades. Ao ABRIR, a tela
// NÃO verifica na rede — reaproveita o resultado da verificação feita ao abrir o
// app (`update-status`, que também liga o badge no menu). Só o botão "Buscar
// atualizações" força uma nova verificação. "Atualizar agora" abre a janela de
// novidades (OTA) com o delta + progresso.
import { invoke } from "@tauri-apps/api/core";
import { loadNovidades } from "./novidades";
import { checkUpdateStatus, getUpdateStatus, type UpdateStatus } from "./update-status";

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const REPO_URL = "https://github.com/wzuqui/ai-usage-tray-agent";

// Reflete um `UpdateStatus` na tela: versão instalada, mensagem e qual botão
// aparece — "Atualizar agora" (há update) ou "Buscar atualizações" (sem
// update/erro).
function renderUpdateStatus(st: UpdateStatus): void {
  const msg = $("sobre-update-msg");
  const checkBtn = $<HTMLButtonElement>("sobre-check");
  const updateBtn = $<HTMLButtonElement>("sobre-update");
  $("sobre-appVersion").textContent = st.currentVersion || "—";
  if (st.available && st.newVersion) {
    msg.textContent = `Atualização disponível — ${st.currentVersion} → ${st.newVersion}`;
    msg.className = "sobre-status avail";
    checkBtn.classList.add("hide");
    updateBtn.classList.remove("hide");
  } else if (st.error) {
    msg.textContent = "Não foi possível verificar atualizações.";
    msg.className = "sobre-status err";
    checkBtn.classList.remove("hide");
    updateBtn.classList.add("hide");
  } else {
    msg.textContent = "✓ Você está na versão mais recente.";
    msg.className = "sobre-status ok";
    checkBtn.classList.remove("hide");
    updateBtn.classList.add("hide");
  }
}

// Botão "Buscar atualizações": força uma nova verificação (rede) e re-renderiza
// (atualizando também o badge do menu, via `update-status`).
async function recheck(): Promise<void> {
  const msg = $("sobre-update-msg");
  const checkBtn = $<HTMLButtonElement>("sobre-check");
  msg.textContent = "Verificando atualizações…";
  msg.className = "sobre-status";
  checkBtn.disabled = true;
  try {
    renderUpdateStatus(await checkUpdateStatus());
  } finally {
    checkBtn.disabled = false;
  }
}

// Mostra o status já conhecido (cache) ou aguarda a verificação em andamento,
// SEM disparar uma nova. Se nada foi iniciado, mantém o texto neutro do HTML.
async function showKnownStatus(): Promise<void> {
  const known = getUpdateStatus();
  if (known) renderUpdateStatus(await known);
}

let wired = false;
let loaded = false;

export function initSobre(): void {
  if (!wired) {
    wired = true;
    $("sobre-check").addEventListener("click", () => void recheck());
    $("sobre-update").addEventListener("click", () => void invoke("open_update_window"));
    $("sobre-repo").addEventListener("click", (e) => {
      e.preventDefault();
      void invoke("open_external", { url: REPO_URL });
    });
  }
  // Não verifica na rede ao abrir: reaproveita a verificação feita ao abrir o app
  // (o botão "Buscar atualizações" refaz sob demanda). As Novidades carregam uma
  // vez por sessão.
  void showKnownStatus();
  if (!loaded) {
    loaded = true;
    void loadNovidades();
  }
}
