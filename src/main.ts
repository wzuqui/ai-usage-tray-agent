// Shell do app: troca entre as seções (Dashboard / Configurações) pelo menu
// lateral. Janela única, aberta pelo item "Abrir" do tray.
import { initDashboard, loadDashboard } from "./dashboard";
import { initSettings } from "./settings";

function activate(view: string): void {
  document.querySelectorAll(".nav-item").forEach((b) =>
    b.classList.toggle("on", (b as HTMLElement).dataset.view === view));
  document.querySelectorAll(".view").forEach((s) =>
    s.classList.toggle("on", s.id === "view-" + view));
  if (view === "dashboard") void loadDashboard();
  else if (view === "settings") initSettings();
}

document.querySelectorAll(".nav-item").forEach((b) =>
  b.addEventListener("click", () => activate((b as HTMLElement).dataset.view ?? "dashboard")));

// Ao reabrir a janela (item "Abrir" do tray), recarrega o dashboard se for a
// seção ativa, para não mostrar dados velhos. Barato: o backend tem cache.
window.addEventListener("focus", () => {
  if (document.getElementById("view-dashboard")?.classList.contains("on")) void loadDashboard();
});

initDashboard();