// Tela "Sobre" (última opção do menu): versão instalada, verificação de
// atualização (inline, sem diálogo) e o histórico de Novidades. A versão e o
// status vêm de um único `check_update_status`; "Atualizar agora" abre a janela
// de novidades (OTA) com o delta + progresso.
import { invoke } from "@tauri-apps/api/core";
import { loadNovidades } from "./novidades";

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const REPO_URL = "https://github.com/wzuqui/ai-usage-tray-agent";

interface UpdateStatus {
  available: boolean;
  currentVersion: string;
  newVersion: string | null;
  error: string | null;
}

// Verifica atualização (sem abrir janela/diálogo), preenche a versão instalada e
// ajusta a mensagem + qual botão aparece: "Atualizar agora" (há update) OU
// "Buscar atualizações" (sem update/erro).
async function refreshUpdateStatus(): Promise<void> {
  const msg = $("sobre-update-msg");
  const checkBtn = $<HTMLButtonElement>("sobre-check");
  const updateBtn = $<HTMLButtonElement>("sobre-update");
  msg.textContent = "Verificando atualizações…";
  msg.className = "sobre-status";
  checkBtn.disabled = true;
  try {
    const st = await invoke<UpdateStatus>("check_update_status");
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
  } catch {
    msg.textContent = "Não foi possível verificar atualizações.";
    msg.className = "sobre-status err";
    checkBtn.classList.remove("hide");
    updateBtn.classList.add("hide");
  } finally {
    checkBtn.disabled = false;
  }
}

let wired = false;
let loaded = false;

export function initSobre(): void {
  if (!wired) {
    wired = true;
    $("sobre-check").addEventListener("click", () => void refreshUpdateStatus());
    $("sobre-update").addEventListener("click", () => void invoke("open_update_window"));
    $("sobre-repo").addEventListener("click", (e) => {
      e.preventDefault();
      void invoke("open_external", { url: REPO_URL });
    });
  }
  // Carrega uma vez por sessão (são chamadas de rede); os botões refazem sob demanda.
  if (!loaded) {
    loaded = true;
    void refreshUpdateStatus();
    void loadNovidades();
  }
}
