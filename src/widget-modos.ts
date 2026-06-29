// Modos alternativos de exibição do widget da área de trabalho, além do
// "completo" (cards com barras, em widget.ts): "minimo" (uma linha por
// provedor) e "anelduplo" (anéis de progresso concêntricos). Escolhidos pela
// preferência `modo` da aba Configurações → Widget. Funções puras que retornam
// HTML com classes, no mesmo padrão de `renderProvider`/`windowBlock` do
// widget.ts; usam os helpers compartilhados de ./usage-format.
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

/// Como exibir o reset: "restante" (regressivo ao vivo), "exato" (hora/data) ou
/// "nenhum" (oculta o reset em todos os modos).
export type ResetMode = "restante" | "exato" | "nenhum";

/// Normaliza o `formatoReset` vindo da config. "nenhum" é a opção que oculta.
export function parseResetMode(value: string | undefined): ResetMode {
  const v = (value ?? "").trim().toLowerCase();
  if (v === "exato") return "exato";
  if (v === "nenhum" || v === "none" || v === "off") return "nenhum";
  return "restante";
}

/// Texto do reset respeitando o formato. Retorna "" quando "nenhum" ou sem dado.
/// No modo "restante" devolve o <span class="w-remain" data-reset> que o tick()
/// do widget.ts atualiza ao vivo a cada 1s; no "exato" devolve texto estático.
export function resetInline(resetIso: string | null | undefined, mode: ResetMode): string {
  if (mode === "nenhum" || !resetIso) return "";
  if (mode === "exato") return fmtResetClock(resetIso);
  return `<span class="w-remain" data-reset="${escapeHtml(resetIso)}">${fmtRemaining(resetIso)}</span>`;
}

// ----------------------------------------------------------------------------
// Modo "minimo": uma linha por provedor — ícone + nome + % por janela (sem
// rótulos 5h/7d), com o reset entre parênteses após cada % (some no "nenhum").
// ----------------------------------------------------------------------------
function minWindow(
  pct: number | undefined | null,
  resetIso: string | null | undefined,
  mode: ResetMode,
): string {
  if (pct === undefined || pct === null) return "";
  const reset = resetInline(resetIso, mode);
  const resetSpan = reset ? `<span class="wmin-reset">(${reset})</span>` : "";
  return `<span class="wmin-item">
    <span class="wmin-pct" style="color:${barColor(pct)}">${pctText(pct)}%</span>${resetSpan}
  </span>`;
}

export function renderProviderMinimo(
  label: string,
  prov: ProviderUsage,
  mostra: boolean,
  janelas: { sessao: boolean; semanal: boolean },
  mode: ResetMode,
): string | null {
  if (!mostra || !prov.habilitado) return null;
  const icon = label === "Codex" ? iconCodex() : ICON_CLAUDE;
  const head = `${icon}<span class="wprov-name">${label}</span>`;
  const m = prov.metric;
  if (!m) return `<div class="wprov wprov-min">${head}<span class="wprov-note">Coletando…</span></div>`;
  if (m.status === "erro" || m.erro) {
    return `<div class="wprov wprov-min error">${head}<span class="wprov-note err">erro</span></div>`;
  }
  const items = [
    janelas.sessao ? minWindow(m.uso_percentual, m.reset_em, mode) : "",
    janelas.semanal ? minWindow(m.uso_percentual_7d, m.reset_em_7d, mode) : "",
  ].join("");
  return `<div class="wprov wprov-min">${head}<span class="wmin-vals">${items}</span></div>`;
}

// ----------------------------------------------------------------------------
// Modo "anelduplo": anéis concêntricos por provedor (sessão = anel externo,
// semanal = anel interno). Com 1 janela só, desenha apenas o anel INTERNO e
// encolhe o SVG (widget menor). Ícone centralizado; legenda à direita com a %
// maior e o reset entre parênteses (some no "nenhum").
//
// O SVG gira -90° para o progresso começar no topo; stroke-linecap:round dá as
// pontas arredondadas. dasharray = (pct/100 * circunferência) + " " + circunf.
// Raios: externo r=31 (circ 194.78), interno r=22 (circ 138.23), stroke 6.
// ----------------------------------------------------------------------------
interface Ring { r: number; circ: number; }
const RING_OUTER: Ring = { r: 31, circ: 194.78 };
const RING_INNER: Ring = { r: 22, circ: 138.23 };

function ringDash(pct: number, circ: number): string {
  const p = Math.max(0, Math.min(100, pct));
  return `${((p / 100) * circ).toFixed(2)} ${circ}`;
}

export function renderProviderAnelDuplo(
  label: string,
  prov: ProviderUsage,
  mostra: boolean,
  janelas: { sessao: boolean; semanal: boolean },
  mode: ResetMode,
): string | null {
  if (!mostra || !prov.habilitado) return null;
  // Sem o nome do provedor: o ícone (spark do Claude / logo do Codex) já o
  // identifica, então a legenda fica só com os anéis e as porcentagens.
  const icon = label === "Codex" ? iconCodex() : ICON_CLAUDE;
  const m = prov.metric;
  if (!m) return `<div class="wprov wprov-duplo">${icon}<span class="wprov-note">Coletando…</span></div>`;
  if (m.status === "erro" || m.erro) {
    return `<div class="wprov wprov-duplo error">${icon}<span class="wprov-note err">erro</span></div>`;
  }

  // Janelas presentes, na ordem sessão → semanal (a ordem das linhas e a posição
  // do anel — externo = sessão, interno = semanal — identificam cada janela).
  const wins: Array<{ pct: number; reset: string | null | undefined; ring: Ring }> = [];
  if (janelas.sessao && m.uso_percentual != null) {
    wins.push({ pct: m.uso_percentual, reset: m.reset_em, ring: RING_OUTER });
  }
  if (janelas.semanal && m.uso_percentual_7d != null) {
    wins.push({ pct: m.uso_percentual_7d, reset: m.reset_em_7d, ring: RING_INNER });
  }

  const single = wins.length === 1;
  // Uma janela só → usa o anel INTERNO e encolhe o SVG.
  if (single) wins[0].ring = RING_INNER;
  const size = single ? 54 : 76;
  const c = size / 2;

  const bgCircles = wins
    .map((w) => `<circle cx="${c}" cy="${c}" r="${w.ring.r}" fill="none" stroke="rgba(0,0,0,.4)" stroke-width="6"/>`)
    .join("");
  const fgCircles = wins
    .map((w) =>
      `<circle cx="${c}" cy="${c}" r="${w.ring.r}" fill="none" stroke="${barColor(w.pct)}" ` +
      `stroke-width="6" stroke-linecap="round" stroke-dasharray="${ringDash(w.pct, w.ring.circ)}"/>`)
    .join("");

  const svg =
    `<span class="wduplo-rings" style="width:${size}px;height:${size}px">` +
      `<svg width="${size}" height="${size}" viewBox="0 0 ${size} ${size}" style="transform:rotate(-90deg)">` +
        bgCircles + fgCircles +
      `</svg>` +
      `<span class="wduplo-icon">${icon}</span>` +
    `</span>`;

  const legend = wins
    .map((w) => {
      const reset = resetInline(w.reset, mode);
      const resetSpan = reset ? `<span class="wduplo-reset">(${reset})</span>` : "";
      return `<span class="wduplo-leg-row">
        <span class="wduplo-dot" style="background:${barColor(w.pct)}"></span>
        <span class="wduplo-pct" style="color:${barColor(w.pct)}">${pctText(w.pct)}%</span>${resetSpan}
      </span>`;
    })
    .join("");

  return `<div class="wprov wprov-duplo">
    ${svg}
    <span class="wduplo-info">${legend}</span>
  </div>`;
}
