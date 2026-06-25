// Shell do app: troca entre as seções (Uso atual / Dashboard Claude /
// Configurações) pelo menu lateral. Janela única, aberta pelo clique esquerdo
// no tray ou pelo item "Abrir". A seção padrão é "Uso atual".
import { initCodexDashboard, loadCodexDashboard } from "./codex-dashboard";
import { initDashboard, loadDashboard } from "./dashboard";
import { initEnvio, loadEnvio } from "./envio";
import { initSettings } from "./settings";
import { initSobre } from "./sobre";
import { checkUpdateStatus } from "./update-status";
import { initUsage, loadUsage } from "./usage";

function activate(view: string): void {
  document.querySelectorAll(".nav-item").forEach((b) =>
    b.classList.toggle("on", (b as HTMLElement).dataset.view === view));
  document.querySelectorAll(".view").forEach((s) =>
    s.classList.toggle("on", s.id === "view-" + view));
  if (view === "envio") initEnvio();
  else if (view === "usage") void loadUsage();
  else if (view === "dashboard") initDashboard();
  else if (view === "codex-dashboard") initCodexDashboard();
  else if (view === "settings") initSettings();
  else if (view === "sobre") initSobre();
}

document.querySelectorAll(".nav-item").forEach((b) =>
  b.addEventListener("click", () => activate((b as HTMLElement).dataset.view ?? "usage")));

// Ao reabrir a janela (clique no tray / item "Abrir"), recarrega a seção ativa
// para não mostrar dados velhos. Ambas as cargas são baratas (snapshot/cache no
// backend).
window.addEventListener("focus", () => {
  if (document.getElementById("view-envio")?.classList.contains("on")) void loadEnvio();
  else if (document.getElementById("view-usage")?.classList.contains("on")) void loadUsage();
  else if (document.getElementById("view-dashboard")?.classList.contains("on")) void loadDashboard();
  else if (document.getElementById("view-codex-dashboard")?.classList.contains("on")) void loadCodexDashboard();
});

initUsage();

// Ao abrir a janela, verifica atualização (uma vez) — se houver, o item "Sobre"
// do menu ganha o badge "Atualização disponível". A tela "Sobre" reaproveita
// esse resultado (não re-verifica ao abrir).
void checkUpdateStatus();