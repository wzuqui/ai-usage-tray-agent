use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

mod usage_dashboard;

#[cfg(target_os = "windows")]
mod taskbar_widget;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, State, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const TRAY_ID: &str = "main-tray";
const APP_NAME_WINDOWS: &str = "AiUsageTrayAgent";
#[cfg(target_os = "linux")]
const APP_NAME_LINUX: &str = "ai-usage-tray-agent";

// `default` no nivel do container faz com que qualquer campo ausente no JSON
// seja preenchido com o valor de `Default` em vez de falhar a desserializacao.
// Combinado com a normalizacao em `load_or_create_config`, isso garante que um
// `config.json` antigo (sem campos novos) seja migrado e reescrito com os
// padroes na inicializacao, sem perder os valores ja configurados.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AppConfig {
    usuario: String,
    intervalo_segundos: u64,
    loki: LokiConfig,
    providers: ProvidersConfig,
    barra_tarefas: TaskbarConfig,
    widget: WidgetConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TaskbarConfig {
    /// Lado da barra onde o widget e' ancorado: "direita" (padrao) ou
    /// "esquerda". O calculo que "adivinha" a posicao e' espelhado conforme o
    /// lado; "esquerda" e' util quando o menu Iniciar esta centralizado (deixa a
    /// ponta esquerda livre). O `deslocamento` continua valendo em ambos. Em
    /// outros sistemas operacionais o campo e' ignorado (widget so existe no
    /// Windows).
    lado: String,
    /// Desloca o widget na barra de tarefas (px). Negativo = para a esquerda;
    /// positivo = para a direita. Util para nao sobrepor toolbars/deskbands
    /// (ex.: atalhos de pasta no Windows 10 -> use um valor negativo).
    deslocamento: i32,
    /// Tamanho da fonte em pontos (padrao 9). Limitado a 6..=24.
    tamanho_fonte: u32,
    /// Cor da fonte: "auto" (padrao, preto/branco conforme a cor da barra) ou um
    /// hex "#RRGGBB" (ex.: "#FFD700"). Valores invalidos voltam a "auto".
    cor_fonte: String,
    /// Como exibir o reset no widget: "restante" (padrao, tempo regressivo ex.:
    /// "2:36h") ou "exato" (hora/data do reset ex.: "19:20" ou "22/06, 19:59").
    formato_reset: String,
    /// Quais janelas mostrar na barra: "ambos" (padrao), "sessao" (so 5h) ou
    /// "semanal" (so 7d). Com uma so' janela, o separador "|" some.
    janelas: String,
}

impl Default for TaskbarConfig {
    fn default() -> Self {
        Self {
            lado: "direita".to_string(),
            deslocamento: 0,
            tamanho_fonte: 9,
            cor_fonte: "auto".to_string(),
            formato_reset: "restante".to_string(),
            janelas: "ambos".to_string(),
        }
    }
}

#[cfg(target_os = "windows")]
impl TaskbarConfig {
    /// `true` se o lado configurado e' a esquerda (aceita variacoes comuns).
    fn lado_esquerdo(&self) -> bool {
        matches!(
            self.lado.trim().to_ascii_lowercase().as_str(),
            "esquerda" | "esquerdo" | "left" | "e"
        )
    }

    /// `true` se o reset deve ser exibido como hora/data exata em vez do tempo
    /// restante (aceita variacoes comuns).
    fn mostrar_hora_reset(&self) -> bool {
        matches!(
            self.formato_reset.trim().to_ascii_lowercase().as_str(),
            "exato" | "exata" | "hora" | "horario" | "data" | "absoluto"
        )
    }

    /// Tamanho da fonte em pontos, com limites sensatos (6..=24); 0/ausente -> 9.
    fn tamanho_fonte_pt(&self) -> i32 {
        let pt = self.tamanho_fonte as i32;
        if pt <= 0 {
            9
        } else {
            pt.clamp(6, 24)
        }
    }

    /// Cor da fonte como `(r, g, b)`, ou `None` para automatico (preto/branco
    /// conforme a cor real da barra). Aceita "#RRGGBB" ou "RRGGBB".
    fn cor_fonte_rgb(&self) -> Option<(u8, u8, u8)> {
        let texto = self.cor_fonte.trim().trim_start_matches('#');
        if texto.is_empty() || texto.eq_ignore_ascii_case("auto") || texto.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&texto[0..2], 16).ok()?;
        let g = u8::from_str_radix(&texto[2..4], 16).ok()?;
        let b = u8::from_str_radix(&texto[4..6], 16).ok()?;
        Some((r, g, b))
    }
}

/// Widget flutuante na area de trabalho (janela `widget`, sem moldura, sempre na
/// frente). Existe em Windows/Linux; ignorado em macOS (transparencia exigiria
/// `macos-private-api`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WidgetConfig {
    /// Exibe o widget na area de trabalho. Padrao desligado.
    habilitado: bool,
    /// Mostra o card do Claude no widget (alem de o provider estar habilitado).
    mostra_claude: bool,
    /// Mostra o card do Codex no widget (alem de o provider estar habilitado).
    mostra_codex: bool,
    /// Caminho do arquivo de imagem/gif usado como fundo. Vazio = sem fundo.
    fundo: String,
    /// Mantem o widget sempre na frente das outras janelas. Padrao ligado.
    sempre_na_frente: bool,
    /// Opacidade do painel em 0..=100 (padrao 90). Deixa o fundo aparecer.
    opacidade: u32,
    /// Quais janelas mostrar nos cards: "ambos" (padrao), "sessao" (so 5h) ou
    /// "semanal" (so 7d).
    janelas: String,
    /// Como exibir o reset nos cards: "restante" (padrao, tempo regressivo) ou
    /// "exato" (hora/data do reset). Igual a opcao da barra de tarefas.
    formato_reset: String,
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self {
            habilitado: false,
            mostra_claude: true,
            mostra_codex: true,
            fundo: String::new(),
            sempre_na_frente: true,
            opacidade: 90,
            janelas: "ambos".to_string(),
            formato_reset: "restante".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct LokiConfig {
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ProvidersConfig {
    codex: CodexConfig,
    claude: ClaudeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CodexConfig {
    habilitado: bool,
    /// Mostra este provider no widget da barra de tarefas (somente Windows).
    /// Em outros sistemas operacionais o campo e lido mas ignorado, pois o
    /// widget da barra so existe no Windows.
    mostra_na_taskbar_windows: bool,
    auth_json_path: String,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            habilitado: true,
            mostra_na_taskbar_windows: true,
            auth_json_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ClaudeConfig {
    habilitado: bool,
    /// Mostra este provider no widget da barra de tarefas (somente Windows).
    /// Em outros sistemas operacionais o campo e lido mas ignorado, pois o
    /// widget da barra so existe no Windows.
    mostra_na_taskbar_windows: bool,
    organization_id: String,
    cookie: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            habilitado: true,
            mostra_na_taskbar_windows: true,
            organization_id: String::new(),
            cookie: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageMetric {
    usuario: String,
    ferramenta: String,
    uso_percentual: f64,
    restante_percentual: f64,
    status: String,
    coletado_em: String,
    reset_em: Option<String>,
    erro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uso_percentual_7d: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restante_percentual_7d: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reset_em_7d: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimePaths {
    config_dir: PathBuf,
    config_file: PathBuf,
    logs_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct RuntimeSnapshot {
    paused: bool,
    last_error: Option<String>,
    last_successful_send_at: Option<String>,
    codex_metric: Option<UsageMetric>,
    claude_metric: Option<UsageMetric>,
}

struct SharedState {
    snapshot: Mutex<RuntimeSnapshot>,
    cycle_lock: Mutex<()>,
    stop: AtomicBool,
    /// Ha' um envio manual ("Enviar agora") em andamento. Evita empilhar uma
    /// thread por clique enquanto um ciclo anterior ainda espera o `cycle_lock`.
    manual_pending: AtomicBool,
}

/// Handles dos itens dinamicos do menu do tray, para atualiza-los no lugar
/// (set_text/set_checked/set_enabled) em vez de reconstruir o menu — assim o
/// menu nao fecha sozinho quando atualizamos a cada ciclo de coleta.
struct TrayMenuItems<R: Runtime> {
    status: MenuItem<R>,
    codex_status: MenuItem<R>,
    claude_status: MenuItem<R>,
    toggle_pause: MenuItem<R>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeAuth {
    openai: Option<OpenAiAccess>,
    tokens: Option<OpenAiTokens>,
}

#[derive(Debug, Deserialize)]
struct OpenAiAccess {
    access: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiTokens {
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsageResponse {
    rate_limit: Option<OpenAiRateLimit>,
}

#[derive(Debug, Deserialize)]
struct OpenAiRateLimit {
    primary_window: Option<OpenAiPrimaryWindow>,
    secondary_window: Option<OpenAiSecondaryWindow>,
}

#[derive(Debug, Deserialize)]
struct OpenAiPrimaryWindow {
    used_percent: Option<f64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiSecondaryWindow {
    used_percent: Option<f64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsageResponse {
    five_hour: Option<ClaudeFiveHour>,
    seven_day: Option<ClaudeSevenDay>,
}

#[derive(Debug, Deserialize)]
struct ClaudeFiveHour {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeSevenDay {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            usuario: String::new(),
            intervalo_segundos: 10,
            loki: LokiConfig::default(),
            providers: ProvidersConfig::default(),
            barra_tarefas: TaskbarConfig::default(),
            widget: WidgetConfig::default(),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            // Persiste POSICAO e SIZE do widget (o usuario pode redimensiona-lo;
            // o widget.ts so' auto-ajusta a altura ate' o primeiro resize manual).
            // A janela `main` fica de fora (negada) e continua centralizando sob
            // demanda.
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&["main"])
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::SIZE,
                )
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            usage_dashboard::get_stats,
            get_settings,
            save_settings,
            get_usage,
            force_collect,
            get_widget_state,
            read_widget_background,
            pick_widget_background,
            show_app_menu
        ])
        .setup(|app| {
            // Janela unica do app (Dashboard + Configuracoes) e' criada sob demanda
            // em show_main_window (tray-only). Fechar pela X destroi a janela e
            // libera o WebView2 (~140 MB); reabrir recria. O app continua vivo no
            // tray porque prevent_exit no run loop impede a saida ao fechar a ultima
            // janela.

            let paths = ensure_storage()?;
            // Garante que o config.json exista e esteja normalizado (campos
            // novos preenchidos com o padrao) ja na inicializacao. A preferencia
            // de exibir na barra de tarefas e lida da config sob demanda.
            load_or_create_config(&paths)?;

            let shared = Arc::new(SharedState {
                snapshot: Mutex::new(RuntimeSnapshot::default()),
                cycle_lock: Mutex::new(()),
                stop: AtomicBool::new(false),
                manual_pending: AtomicBool::new(false),
            });

            app.manage(paths.clone());
            app.manage(shared.clone());

            // Autostart: liga por padrao na primeira execucao (marcada por um
            // arquivo). Nas execucoes seguintes, se continuar ligado, reaplica
            // para manter o caminho do executavel atualizado; se o usuario tiver
            // desligado pelo menu, fica desligado.
            let autostart_marker = paths.config_dir.join("autostart_initialized");
            if !autostart_marker.exists() {
                let _ = app.autolaunch().enable();
                let _ = fs::write(&autostart_marker, "1");
            } else if app.autolaunch().is_enabled().unwrap_or(false) {
                let _ = app.autolaunch().enable();
            }

            create_tray(app)?;

            #[cfg(target_os = "windows")]
            {
                // Clicar no widget da barra abre a janela do app (mesma acao do
                // clique esquerdo no tray). A thread do widget nao tem o
                // AppHandle, entao registramos um callback; ele despacha para a
                // main thread, onde as operacoes de janela sao seguras.
                let app_handle = app.handle().clone();
                taskbar_widget::set_on_activate(move || {
                    let handle = app_handle.clone();
                    let _ = app_handle.run_on_main_thread(move || show_main_window(&handle));
                });

                // Clique direito no widget da barra: roteia o item escolhido para
                // o mesmo tratador do menu do tray, na main thread.
                let app_handle_menu = app.handle().clone();
                taskbar_widget::set_on_menu_command(move |id| {
                    let handle = app_handle_menu.clone();
                    let id = id.to_string();
                    let _ = app_handle_menu
                        .run_on_main_thread(move || handle_menu_event(&handle, &id));
                });

                taskbar_widget::start();
            }

            // Abre o widget no boot se estiver habilitado na config.
            if let Ok(config) = load_or_create_config(&paths) {
                apply_widget(app.handle(), &config);
            }

            refresh_tray(app.handle(), &shared)?;
            start_worker(app.handle().clone(), paths.clone(), shared.clone());

            Ok(())
        })
        .on_menu_event(|app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    // Saida disparada por fechar a ultima janela (X do dashboard):
                    // o app e' tray-only, entao impede a saida e segue rodando.
                    api.prevent_exit();
                } else if let Some(state) = app_handle.try_state::<Arc<SharedState>>() {
                    // Saida real (menu "Sair" -> app.exit): sinaliza o worker.
                    state.stop.store(true, Ordering::Relaxed);
                }
            }
        });
}

/// Corpo do POST de configuracoes: o config.json completo mais a preferencia de
/// autostart (que nao mora no config.json, e' gerenciada pelo plugin).
#[derive(Debug, Deserialize)]
struct SaveSettings {
    config: AppConfig,
    #[serde(default)]
    autostart: bool,
}

/// Estado exposto ao painel de configuracoes: o config.json (normalizado), a
/// preferencia de autostart, o SO e o rotulo do autostart (para a UI).
fn settings_value<R: Runtime>(app: &AppHandle<R>, paths: &RuntimePaths) -> Value {
    let config = load_or_create_config(paths).unwrap_or_default();
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);

    let mut value = json!({
        "autostart": autostart,
        "os": std::env::consts::OS,
        "autostartLabel": autostart_label(),
    });
    value["config"] = serde_json::to_value(&config).unwrap_or(Value::Null);
    value
}

fn autostart_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Iniciar com o Windows"
    } else {
        "Iniciar com o sistema"
    }
}

/// Liga/desliga o autostart so' quando o estado pedido difere do atual.
fn apply_autostart<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    let manager = app.autolaunch();
    let currently = manager.is_enabled().unwrap_or(false);
    if enabled != currently {
        let _ = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
    }
}

/// Le o estado atual (config + autostart) para preencher o painel.
#[tauri::command]
fn get_settings(app: AppHandle, paths: State<'_, RuntimePaths>) -> Value {
    settings_value(&app, paths.inner())
}

/// Grava o config.json e aplica o autostart, devolvendo o estado ja' normalizado.
/// A normalizacao (clamp de intervalo/fonte, validacao de cor) acontece na
/// releitura; o worker detecta a mudanca pelo mtime e aplica tray/barra em ~1s.
#[tauri::command]
fn save_settings(
    app: AppHandle,
    paths: State<'_, RuntimePaths>,
    settings: SaveSettings,
) -> Result<Value, String> {
    write_config(paths.inner(), &settings.config)
        .map_err(|error| format!("falha ao salvar config.json: {error}"))?;
    apply_autostart(&app, settings.autostart);
    Ok(settings_value(&app, paths.inner()))
}

/// Estado de uso exposto a' tela "Uso atual": as metricas atuais de cada
/// provider (a mesma fonte do tray e da barra de tarefas), mais se cada um esta'
/// habilitado e se a coleta esta' pausada. Nao faz rede: le' apenas o snapshot
/// ja' coletado pelo worker.
fn usage_value(paths: &RuntimePaths, shared: &Arc<SharedState>) -> Value {
    let snapshot = shared.snapshot.lock().unwrap().clone();
    let config = load_or_create_config(paths).unwrap_or_default();
    json!({
        "paused": snapshot.paused,
        "lastError": snapshot.last_error,
        "claude": {
            "habilitado": config.providers.claude.habilitado,
            "metric": snapshot.claude_metric,
        },
        "codex": {
            "habilitado": config.providers.codex.habilitado,
            "metric": snapshot.codex_metric,
        },
    })
}

/// Le' o uso atual (snapshot) para a tela "Uso atual". Barato e sem rede; pode
/// ser chamado ao abrir/focar a janela.
#[tauri::command]
fn get_usage(paths: State<'_, RuntimePaths>, shared: State<'_, Arc<SharedState>>) -> Value {
    usage_value(paths.inner(), shared.inner())
}

/// Forca uma coleta nova ("Atualizar agora") e devolve o uso ja' atualizado, para
/// a tela mostrar o resultado assim que termina. Roda em `spawn_blocking` para nao
/// travar a main thread: a coleta usa rede sincrona (ate' ~15s de timeout). O erro
/// do ciclo, se houver, ja' fica refletido no proprio snapshot (status/erro por
/// provider). Respeita a pausa: com a coleta pausada, atualiza os dados/UI **sem**
/// enviar ao Loki (o envio so' ocorre com a coleta ativa ou no "Enviar agora").
#[tauri::command]
async fn force_collect(app: AppHandle) -> Value {
    tauri::async_runtime::spawn_blocking(move || {
        let paths = app.state::<RuntimePaths>().inner().clone();
        let shared = app.state::<Arc<SharedState>>().inner().clone();
        let send = !shared.snapshot.lock().unwrap().paused;
        let _ = run_collection_cycle(&app, &paths, &shared, send);
        usage_value(&paths, &shared)
    })
    .await
    .unwrap_or_else(|error| json!({ "error": error.to_string() }))
}

/// Estado para o widget da area de trabalho: preferencias do widget mais as
/// metricas atuais (o mesmo snapshot da tela "Uso atual"). Barato e sem rede.
fn widget_state_value(paths: &RuntimePaths, shared: &Arc<SharedState>) -> Value {
    let snapshot = shared.snapshot.lock().unwrap().clone();
    let config = load_or_create_config(paths).unwrap_or_default();
    let widget = &config.widget;
    json!({
        "habilitado": widget.habilitado,
        "mostraClaude": widget.mostra_claude,
        "mostraCodex": widget.mostra_codex,
        "fundo": widget.fundo,
        "opacidade": widget.opacidade,
        "janelas": widget.janelas,
        "formatoReset": widget.formato_reset,
        "sempreNaFrente": widget.sempre_na_frente,
        "paused": snapshot.paused,
        "claude": {
            "habilitado": config.providers.claude.habilitado,
            "metric": snapshot.claude_metric,
        },
        "codex": {
            "habilitado": config.providers.codex.habilitado,
            "metric": snapshot.codex_metric,
        },
    })
}

/// Le' o estado do widget (preferencias + uso). Chamado periodicamente pela
/// janela do widget; barato e sem rede.
#[tauri::command]
fn get_widget_state(paths: State<'_, RuntimePaths>, shared: State<'_, Arc<SharedState>>) -> Value {
    widget_state_value(paths.inner(), shared.inner())
}

/// Le' o arquivo de fundo configurado e devolve um data URL base64 para exibir no
/// widget (funciona para imagem e gif). `None` quando nao ha' fundo ou o arquivo
/// nao pode ser lido. So' e' chamado quando o caminho do fundo muda.
#[tauri::command]
fn read_widget_background(paths: State<'_, RuntimePaths>) -> Option<String> {
    let config = load_or_create_config(paths.inner()).ok()?;
    let path = config.widget.fundo.trim();
    if path.is_empty() {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let mime = mime_from_path(path);
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

/// Mime a partir da extensao do arquivo de fundo (imagens e gif).
fn mime_from_path(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// Abre o seletor de arquivo nativo para escolher a imagem/gif de fundo do
/// widget. Devolve o caminho escolhido, ou `None` se o usuario cancelar. Quem
/// persiste e' o "Salvar" das Configuracoes (mesmo fluxo dos demais campos).
#[tauri::command]
fn pick_widget_background(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .add_filter("Imagens e GIFs", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
        .blocking_pick_file()
        .and_then(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().to_string())
}

/// Abre o menu do app (mesmos itens do tray) na posicao do cursor. Chamado pelo
/// clique direito no widget da area de trabalho; reusa o mesmo menu nativo do
/// widget da barra (que ja' roteia o item escolhido para `handle_menu_event`).
/// So' Windows: em outros SOs e' um no-op. Roda na main thread, que tem o loop
/// de mensagens necessario para o menu modal.
#[tauri::command]
fn show_app_menu(app: AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let _ = app.run_on_main_thread(|| unsafe { taskbar_widget::show_context_menu() });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
    }
}

/// Cria a janela do widget (sem moldura, sempre na frente, arrastavel) carregando
/// `widget.html`. Posicao/tamanho sao restaurados pelo plugin window-state quando
/// houver estado salvo. So' existe em Windows/Linux (transparencia em macOS
/// exigiria `macos-private-api`).
#[cfg(not(target_os = "macos"))]
fn show_widget_window<R: Runtime>(app: &AppHandle<R>, config: &AppConfig) {
    if app.get_webview_window("widget").is_some() {
        return;
    }
    let result = WebviewWindowBuilder::new(app, "widget", WebviewUrl::App("widget.html".into()))
        .title("Widget de uso")
        .inner_size(320.0, 180.0)
        .min_inner_size(160.0, 120.0)
        .decorations(false)
        .transparent(true)
        .skip_taskbar(true)
        .always_on_top(config.widget.sempre_na_frente)
        // Redimensionavel pelo usuario; o tamanho e' salvo (window-state). Na
        // primeira vez, o proprio widget ajusta a altura ao conteudo.
        .resizable(true)
        // Posicao inicial no centro; restauramos a ultima posicao/tamanho
        // salvos logo abaixo, quando houver estado.
        .center()
        .build();
    match result {
        Ok(window) => {
            use tauri_plugin_window_state::{StateFlags, WindowExt};
            // Reaplica a ultima posicao/tamanho salvos (no-op na primeira vez).
            let _ = window.restore_state(StateFlags::POSITION | StateFlags::SIZE);
        }
        Err(error) => handle_runtime_error(app, &format!("Falha ao abrir o widget: {error}")),
    }
}

/// Aplica a config do widget: cria/destroi a janela conforme `habilitado` e
/// atualiza o "sempre na frente". Pode ser chamada da thread do worker, entao as
/// operacoes de janela sao despachadas para a main thread.
#[cfg(not(target_os = "macos"))]
fn apply_widget<R: Runtime>(app: &AppHandle<R>, config: &AppConfig) {
    let app = app.clone();
    let config = config.clone();
    let _ = app.clone().run_on_main_thread(move || {
        match (config.widget.habilitado, app.get_webview_window("widget")) {
            (true, Some(window)) => {
                let _ = window.set_always_on_top(config.widget.sempre_na_frente);
            }
            (true, None) => show_widget_window(&app, &config),
            (false, Some(window)) => {
                let _ = window.destroy();
            }
            (false, None) => {}
        }
    });
}

#[cfg(target_os = "macos")]
fn apply_widget<R: Runtime>(_app: &AppHandle<R>, _config: &AppConfig) {}

fn create_tray<R: Runtime>(app: &mut tauri::App<R>) -> tauri::Result<()> {
    let (menu, handles) = build_tray_menu(app.handle(), &RuntimeSnapshot::default())?;
    app.manage(handles);

    // Clique esquerdo abre o app; clique direito abre o menu (padrao do Windows).
    // `show_menu_on_left_click(false)` impede o menu no clique esquerdo; o menu no
    // clique direito continua sendo o comportamento padrao da bandeja.
    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("AiUsageTrayAgent")
        .build(app)?;

    Ok(())
}

/// Texto do item de status conforme o estado atual.
fn tray_status_text(snapshot: &RuntimeSnapshot) -> String {
    if snapshot.paused {
        "Status: coleta pausada".to_string()
    } else if let Some(error) = &snapshot.last_error {
        format!("Status: erro - {}", truncate(error, 64))
    } else {
        "Status: coleta ativa".to_string()
    }
}

fn tray_pause_label(snapshot: &RuntimeSnapshot) -> &'static str {
    if snapshot.paused {
        "Retomar coleta"
    } else {
        "Pausar coleta"
    }
}

fn build_tray_menu<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: &RuntimeSnapshot,
) -> tauri::Result<(Menu<R>, TrayMenuItems<R>)> {
    let status_text = tray_status_text(snapshot);
    let codex_text = format!("Codex: {}", metric_text(snapshot.codex_metric.as_ref()));
    let claude_text = format!("Claude: {}", metric_text(snapshot.claude_metric.as_ref()));
    let pause_label = tray_pause_label(snapshot);

    let status_item = MenuItem::with_id(app, "status", status_text, false, None::<&str>)?;
    let codex_item = MenuItem::with_id(app, "codex_status", codex_text, false, None::<&str>)?;
    let claude_item = MenuItem::with_id(app, "claude_status", claude_text, false, None::<&str>)?;
    let open_app_item = MenuItem::with_id(app, "open_app", "Abrir", true, None::<&str>)?;
    let open_config_item =
        MenuItem::with_id(app, "open_config", "Abrir config.json", true, None::<&str>)?;
    let open_logs_item =
        MenuItem::with_id(app, "open_logs", "Abrir pasta de logs", true, None::<&str>)?;
    let send_now_item = MenuItem::with_id(app, "send_now", "Enviar agora", true, None::<&str>)?;
    let toggle_pause_item =
        MenuItem::with_id(app, "toggle_pause", pause_label, true, None::<&str>)?;

    let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;

    let separator_info = PredefinedMenuItem::separator(app)?;
    let separator_actions = PredefinedMenuItem::separator(app)?;

    let items: Vec<&dyn IsMenuItem<R>> = vec![
        &status_item,
        &codex_item,
        &claude_item,
        &separator_info,
        &open_app_item,
        &open_config_item,
        &open_logs_item,
        &send_now_item,
        &toggle_pause_item,
        &separator_actions,
        &quit_item,
    ];

    let menu = Menu::with_items(app, &items)?;
    drop(items); // encerra os borrows antes de mover os itens para os handles

    let handles = TrayMenuItems {
        status: status_item,
        codex_status: codex_item,
        claude_status: claude_item,
        toggle_pause: toggle_pause_item,
    };

    Ok((menu, handles))
}

/// Atualiza os itens dinamicos do menu do tray no lugar, sem reconstruir o menu
/// (reconstruir fecharia o menu aberto).
///
/// As atualizacoes sao postadas como uma unica tarefa na main thread sem
/// esperar o resultado. Isso evita que a thread do worker bloqueie enquanto o
/// menu popup esta aberto (a main thread fica no loop modal do menu ate fechar).
fn update_tray_menu<R: Runtime>(app: &AppHandle<R>, snapshot: &RuntimeSnapshot) {
    let status = tray_status_text(snapshot);
    let codex = format!("Codex: {}", metric_text(snapshot.codex_metric.as_ref()));
    let claude = format!("Claude: {}", metric_text(snapshot.claude_metric.as_ref()));
    let pause = tray_pause_label(snapshot).to_string();

    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        let Some(items) = app.try_state::<TrayMenuItems<R>>() else {
            return;
        };
        let _ = items.status.set_text(status);
        let _ = items.codex_status.set_text(codex);
        let _ = items.claude_status.set_text(claude);
        let _ = items.toggle_pause.set_text(pause);
    });
}

/// Exibe e foca a janela unica do app (Dashboard + Configuracoes). Acionada pelo
/// item "Abrir" do tray. Se a janela ja existe, apenas a traz ao foco (desfazendo
/// a minimizacao); senao, a cria sob demanda. Fechar a janela a destroi (libera o
/// WebView2), entao a proxima abertura recai no caminho de criacao.
fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }

    let result = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("AiUsageTrayAgent")
        .inner_size(960.0, 660.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .resizable(true)
        .decorations(true)
        .build();
    match result {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(error) => handle_runtime_error(app, &format!("Falha ao abrir a janela: {error}")),
    }
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, menu_id: &str) {
    match menu_id {
        "open_app" => {
            show_main_window(app);
        }
        "open_config" => {
            if let Some(paths) = app.try_state::<RuntimePaths>() {
                if let Err(error) = open_path(&paths.config_file) {
                    handle_runtime_error(app, &format!("Falha ao abrir config.json: {error}"));
                }
            }
        }
        "open_logs" => {
            if let Some(paths) = app.try_state::<RuntimePaths>() {
                if let Err(error) = open_path(&paths.logs_dir) {
                    handle_runtime_error(app, &format!("Falha ao abrir logs: {error}"));
                }
            }
        }
        "send_now" => {
            trigger_collection(app);
        }
        "toggle_pause" => {
            if let Some(shared) = app.try_state::<Arc<SharedState>>() {
                {
                    let mut snapshot = shared.snapshot.lock().unwrap();
                    snapshot.paused = !snapshot.paused;
                }

                let _ = append_log_line(
                    app.state::<RuntimePaths>().inner(),
                    "info",
                    "Coleta pausada/retomada pelo usuario.",
                    None,
                );
                let _ = refresh_tray(app, &shared);
            }
        }
        "quit" => {
            app.exit(0);
        }
        _ => {}
    }
}

fn trigger_collection<R: Runtime>(app: &AppHandle<R>) {
    let app_handle = app.clone();
    let paths = app.state::<RuntimePaths>().inner().clone();
    let shared = app.state::<Arc<SharedState>>().inner().clone();

    // Coalesce cliques: se ja' ha' um envio manual em andamento, ignora os
    // seguintes (senao cada clique empilharia uma thread bloqueada no cycle_lock).
    if shared.manual_pending.swap(true, Ordering::SeqCst) {
        return;
    }

    let _ = append_log_line(
        &paths,
        "info",
        "Envio manual solicitado pelo usuario.",
        None,
    );

    thread::spawn(move || {
        // "Enviar agora" sempre envia ao Loki, mesmo com a coleta pausada.
        let _ = run_collection_cycle(&app_handle, &paths, &shared, true);
        shared.manual_pending.store(false, Ordering::SeqCst);
    });
}

fn start_worker<R: Runtime + 'static>(
    app: AppHandle<R>,
    paths: RuntimePaths,
    shared: Arc<SharedState>,
) {
    thread::spawn(move || loop {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }

        let mut interval = current_interval(&app, &paths);

        let paused = shared.snapshot.lock().unwrap().paused;
        if !paused {
            let _ = run_collection_cycle(&app, &paths, &shared, true);
        } else {
            let _ = refresh_tray(&app, &shared);
        }

        // Espera ate o proximo ciclo de coleta. A thread ja acordava a cada
        // segundo (para reagir ao stop); aproveitamos esse tick para checar, via
        // mtime, se o config.json foi editado. Se mudou, aplicamos a nova config
        // na hora com refresh_tray (posicao na barra, fonte, cor, lado,
        // visibilidade dos provedores e o proprio intervalo) — SEM disparar um
        // envio extra ao Loki. Assim a edicao vale em ~1s sem reduzir o intervalo
        // de envio. A checagem le so o metadado (stat), nao o conteudo; o arquivo
        // so e' lido/parseado quando o mtime realmente muda.
        let mut last_mtime = config_mtime(&paths);
        let mut elapsed = 0u64;
        while elapsed < interval {
            if shared.stop.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
            elapsed += 1;

            let current = config_mtime(&paths);
            if current != last_mtime {
                let _ = refresh_tray(&app, &shared);
                interval = current_interval(&app, &paths);
                // Re-le o mtime apos aplicar: refresh_tray/normalizacao podem
                // reescrever o arquivo, e nao queremos tratar a propria escrita
                // como uma nova edicao externa.
                last_mtime = config_mtime(&paths);
            }
        }
    });
}

/// Le o intervalo de coleta (segundos) do config.json, com clamp 5..=3600 e
/// fallback de 10s em caso de erro de leitura.
fn current_interval<R: Runtime>(app: &AppHandle<R>, paths: &RuntimePaths) -> u64 {
    match load_or_create_config(paths) {
        Ok(config) => config.intervalo_segundos.clamp(5, 3600),
        Err(error) => {
            handle_runtime_error(app, &format!("Falha ao carregar config: {error}"));
            10
        }
    }
}

/// Data de modificacao do config.json, usada para detectar edicoes externas.
/// `None` quando o arquivo nao pode ser lido (ex.: durante um save atomico do
/// editor); a comparacao com o valor anterior ainda detecta a transicao.
fn config_mtime(paths: &RuntimePaths) -> Option<std::time::SystemTime> {
    fs::metadata(&paths.config_file)
        .and_then(|meta| meta.modified())
        .ok()
}

/// Cliente HTTP compartilhado entre ciclos de coleta. Construido uma unica vez
/// (lazy) para reaproveitar o pool de conexoes/keep-alive e evitar recriar o
/// runtime interno do reqwest a cada ciclo. `Client` e' Arc por dentro, entao
/// clonar e' barato e compartilha o mesmo pool.
fn http_client() -> Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new())
        })
        .clone()
}

/// Coleta as metricas dos providers habilitados e, quando `send` e' `true`, envia
/// cada uma ao Loki. Com `send = false` (ex.: "Atualizar agora" com a coleta
/// pausada) so' atualiza o snapshot/UI, sem trafego ao Loki.
fn run_collection_cycle<R: Runtime>(
    app: &AppHandle<R>,
    paths: &RuntimePaths,
    shared: &Arc<SharedState>,
    send: bool,
) -> Result<(), String> {
    let _lock = shared.cycle_lock.lock().unwrap();
    let config = load_or_create_config(paths).map_err(|error| error.to_string())?;
    let client = http_client();

    let mut had_error = false;

    if config.providers.codex.habilitado {
        match collect_codex_metric(&client, &config) {
            Ok(metric) => {
                update_metric(shared, metric.clone());
                // Com a coleta pausada, "Atualizar agora" so' atualiza os dados/UI;
                // o envio ao Loki ocorre apenas com a coleta ativa ou no "Enviar
                // agora" do tray.
                if send {
                    if let Err(error) = send_metric_to_loki(&client, &config, &metric) {
                        had_error = true;
                        let _ = append_log_line(
                            paths,
                            "error",
                            "Falha ao enviar metrica para o Loki.",
                            Some(json!({
                                "ferramenta": "codex",
                                "error": error
                            })),
                        );
                        handle_runtime_error(app, &error);
                    } else {
                        let _ = append_log_line(
                            paths,
                            "info",
                            "Metrica enviada para o Loki.",
                            Some(json!({
                                "ferramenta": "codex",
                                "uso_percentual": metric.uso_percentual,
                                "uso_percentual_7d": metric.uso_percentual_7d,
                                "status": metric.status,
                                "reset_em": metric.reset_em,
                                "reset_em_7d": metric.reset_em_7d
                            })),
                        );
                        mark_success(shared);
                    }
                }
            }
            Err(error) => {
                had_error = true;
                let metric = build_error_metric(&config.usuario, "codex", &error);
                update_metric(shared, metric);
                let _ = append_log_line(
                    paths,
                    "error",
                    "Falha ao coletar metrica.",
                    Some(json!({
                        "ferramenta": "codex",
                        "error": error
                    })),
                );
                handle_runtime_error(app, &error);
            }
        }
    }

    if config.providers.claude.habilitado {
        match collect_claude_metric(&client, &config) {
            Ok(metric) => {
                update_metric(shared, metric.clone());
                // Com a coleta pausada, "Atualizar agora" so' atualiza os dados/UI;
                // o envio ao Loki ocorre apenas com a coleta ativa ou no "Enviar
                // agora" do tray.
                if send {
                    if let Err(error) = send_metric_to_loki(&client, &config, &metric) {
                        had_error = true;
                        let _ = append_log_line(
                            paths,
                            "error",
                            "Falha ao enviar metrica para o Loki.",
                            Some(json!({
                                "ferramenta": "claude",
                                "error": error
                            })),
                        );
                        handle_runtime_error(app, &error);
                    } else {
                        let _ = append_log_line(
                            paths,
                            "info",
                            "Metrica enviada para o Loki.",
                            Some(json!({
                                "ferramenta": "claude",
                                "uso_percentual": metric.uso_percentual,
                                "uso_percentual_7d": metric.uso_percentual_7d,
                                "status": metric.status,
                                "reset_em": metric.reset_em,
                                "reset_em_7d": metric.reset_em_7d
                            })),
                        );
                        mark_success(shared);
                    }
                }
            }
            Err(error) => {
                had_error = true;
                let metric = build_error_metric(&config.usuario, "claude", &error);
                update_metric(shared, metric);
                let _ = append_log_line(
                    paths,
                    "error",
                    "Falha ao coletar metrica.",
                    Some(json!({
                        "ferramenta": "claude",
                        "error": error
                    })),
                );
                handle_runtime_error(app, &error);
            }
        }
    }

    if !config.providers.codex.habilitado && !config.providers.claude.habilitado {
        handle_runtime_error(app, "Nenhum provider habilitado.");
    } else if !had_error {
        clear_last_error(shared);
    }

    refresh_tray(app, shared).map_err(|error| error.to_string())?;
    Ok(())
}

fn collect_codex_metric(client: &Client, config: &AppConfig) -> Result<UsageMetric, String> {
    let auth_path = config.providers.codex.auth_json_path.trim();
    if auth_path.is_empty() {
        return Err("Caminho do auth.json do Codex nao configurado.".to_string());
    }

    let auth_raw = fs::read_to_string(auth_path)
        .map_err(|error| format!("Falha ao ler auth.json do Codex: {error}"))?;
    let auth: OpenCodeAuth =
        serde_json::from_str(&auth_raw).map_err(|error| format!("auth.json invalido: {error}"))?;
    let token = auth
        .openai
        .and_then(|value| value.access)
        .or_else(|| auth.tokens.and_then(|value| value.access_token))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Campos openai.access ou tokens.access_token nao foram encontrados no auth.json do Codex."
                .to_string()
        })?;

    let response = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("accept", "*/*")
        .header("accept-language", "pt-BR,pt;q=0.9,en;q=0.8")
        .header("authorization", format!("Bearer {token}"))
        .header("cache-control", "no-cache")
        .header("pragma", "no-cache")
        .header("oai-language", "pt-BR")
        .header("x-openai-target-path", "/backend-api/wham/usage")
        .header("x-openai-target-route", "/backend-api/wham/usage")
        .send()
        .map_err(|error| format!("Falha HTTP ao consultar Codex: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Codex retornou status HTTP {}.",
            response.status()
        ));
    }

    let payload: OpenAiUsageResponse = response
        .json()
        .map_err(|error| format!("Falha ao decodificar resposta do Codex: {error}"))?;

    let rate_limit = payload
        .rate_limit
        .ok_or_else(|| "rate_limit nao foi encontrado na resposta do Codex.".to_string())?;

    let primary_window = rate_limit
        .primary_window
        .ok_or_else(|| "rate_limit.primary_window nao foi encontrado na resposta do Codex.".to_string())?;

    let used_percent = primary_window
        .used_percent
        .ok_or_else(|| {
            "rate_limit.primary_window.used_percent nao foi encontrado na resposta do Codex."
                .to_string()
        })?;

    let secondary_used_percent = rate_limit
        .secondary_window
        .as_ref()
        .and_then(|value| value.used_percent);
    let secondary_reset_at = rate_limit
        .secondary_window
        .as_ref()
        .and_then(|value| value.reset_at);

    Ok(UsageMetric {
        usuario: normalized_user(&config.usuario),
        ferramenta: "codex".to_string(),
        uso_percentual: round_percent(used_percent),
        restante_percentual: remaining_percent(used_percent),
        status: "ok".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: primary_window.reset_at.and_then(timestamp_seconds_to_iso),
        erro: None,
        uso_percentual_7d: secondary_used_percent.map(round_percent),
        restante_percentual_7d: secondary_used_percent.map(remaining_percent),
        reset_em_7d: secondary_reset_at.and_then(timestamp_seconds_to_iso),
    })
}

fn collect_claude_metric(client: &Client, config: &AppConfig) -> Result<UsageMetric, String> {
    let organization_id = config.providers.claude.organization_id.trim();
    let cookie = config.providers.claude.cookie.trim();

    if organization_id.is_empty() {
        return Err("Organization ID do Claude nao configurado.".to_string());
    }

    if cookie.is_empty() {
        return Err("Cookie do Claude nao configurado.".to_string());
    }

    let response = client
        .get(format!(
            "https://claude.ai/api/organizations/{organization_id}/usage"
        ))
        .header("accept", "*/*")
        .header("cookie", cookie)
        .header("referer", "https://claude.ai/settings/usage")
        .header(
            "user-agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36",
        )
        .send()
        .map_err(|error| format!("Falha HTTP ao consultar Claude: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Claude retornou status HTTP {}.",
            response.status()
        ));
    }

    let payload: ClaudeUsageResponse = response
        .json()
        .map_err(|error| format!("Falha ao decodificar resposta do Claude: {error}"))?;

    let five_hour = payload
        .five_hour
        .ok_or_else(|| "five_hour nao foi encontrado na resposta do Claude.".to_string())?;
    let utilization = five_hour
        .utilization
        .ok_or_else(|| "five_hour.utilization nao foi encontrado na resposta do Claude.".to_string())?;

    let seven_day_utilization = payload
        .seven_day
        .as_ref()
        .and_then(|value| value.utilization);
    let seven_day_resets_at = payload
        .seven_day
        .as_ref()
        .and_then(|value| value.resets_at.clone());

    Ok(UsageMetric {
        usuario: normalized_user(&config.usuario),
        ferramenta: "claude".to_string(),
        uso_percentual: round_percent(utilization),
        restante_percentual: remaining_percent(utilization),
        status: "ok".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: five_hour.resets_at,
        erro: None,
        uso_percentual_7d: seven_day_utilization.map(round_percent),
        restante_percentual_7d: seven_day_utilization.map(remaining_percent),
        reset_em_7d: seven_day_resets_at,
    })
}

fn send_metric_to_loki(client: &Client, config: &AppConfig, metric: &UsageMetric) -> Result<(), String> {
    if config.loki.url.trim().is_empty() {
        return Err("URL do Loki nao configurada.".to_string());
    }

    let timestamp_nanos = iso_to_nanos(&metric.coletado_em)?;
    let host = hostname::get()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut body = json!({
        "uso_percentual": metric.uso_percentual,
        "restante_percentual": metric.restante_percentual,
        "status": metric.status,
        "reset_em": metric.reset_em
    });

    if let Some(error) = &metric.erro {
        body["erro"] = Value::String(error.clone());
    }

    if let Some(value) = metric.uso_percentual_7d {
        body["uso_percentual_7d"] = json!(value);
    }
    if let Some(value) = metric.restante_percentual_7d {
        body["restante_percentual_7d"] = json!(value);
    }
    if let Some(value) = &metric.reset_em_7d {
        body["reset_em_7d"] = Value::String(value.clone());
    }

    let payload = json!({
        "streams": [
            {
                "stream": {
                    "app": "ai-usage-tray-agent",
                    "usuario": metric.usuario,
                    "ferramenta": metric.ferramenta,
                    "host": host
                },
                "values": [
                    [timestamp_nanos, body.to_string()]
                ]
            }
        ]
    });

    let request = client
        .post(config.loki.url.trim())
        .header("content-type", "application/json")
        .json(&payload);

    let response = request
        .send()
        .map_err(|error| format!("Falha HTTP ao enviar para Loki: {error}"))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Loki retornou status HTTP {}.", response.status()))
    }
}

fn refresh_tray<R: Runtime>(app: &AppHandle<R>, shared: &Arc<SharedState>) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // Le' a config uma unica vez por refresh (em vez de recarregar para o
        // tooltip, a barra de tarefas e o widget separadamente).
        let config = app
            .try_state::<RuntimePaths>()
            .and_then(|paths| load_or_create_config(paths.inner()).ok());

        // Metrica de provider desabilitado nao deve sobreviver no snapshot: senao
        // o tooltip/menu do tray exibiriam um valor obsoleto depois de desligar o
        // provider. O snapshot e' a fonte unica (tambem lida por "Uso atual" e pelo
        // widget, que ja' tratam o estado "desabilitado").
        let snapshot = {
            let mut guard = shared.snapshot.lock().unwrap();
            if let Some(config) = &config {
                if !config.providers.codex.habilitado {
                    guard.codex_metric = None;
                }
                if !config.providers.claude.habilitado {
                    guard.claude_metric = None;
                }
            }
            guard.clone()
        };

        #[cfg(target_os = "windows")]
        {
            tray.set_tooltip(Some(format!(
                "AiUsageTrayAgent\nCodex: {}\nClaude: {}",
                metric_text(snapshot.codex_metric.as_ref()),
                metric_text(snapshot.claude_metric.as_ref())
            )))?;
            taskbar_widget::set_paused(snapshot.paused);
            if let Some(config) = &config {
                taskbar_widget::set_offset(config.barra_tarefas.deslocamento);
                taskbar_widget::set_side(config.barra_tarefas.lado_esquerdo());
                taskbar_widget::set_font_size(config.barra_tarefas.tamanho_fonte_pt());
                taskbar_widget::set_font_color(config.barra_tarefas.cor_fonte_rgb());
                let mostrar_hora = config.barra_tarefas.mostrar_hora_reset();
                let (mostra_sessao, mostra_semanal) =
                    parse_janelas(&config.barra_tarefas.janelas);
                taskbar_widget::set_provider(
                    "codex",
                    config.providers.codex.habilitado
                        && config.providers.codex.mostra_na_taskbar_windows,
                    widget_detail(
                        snapshot.codex_metric.as_ref(),
                        mostrar_hora,
                        mostra_sessao,
                        mostra_semanal,
                    ),
                );
                taskbar_widget::set_provider(
                    "claude",
                    config.providers.claude.habilitado
                        && config.providers.claude.mostra_na_taskbar_windows,
                    widget_detail(
                        snapshot.claude_metric.as_ref(),
                        mostrar_hora,
                        mostra_sessao,
                        mostra_semanal,
                    ),
                );
            }
        }

        #[cfg(target_os = "linux")]
        tray.set_title(Some(format!(
            "C:{} / Cl:{}",
            metric_text(snapshot.codex_metric.as_ref()),
            metric_text(snapshot.claude_metric.as_ref())
        )))?;

        // Aplica a config do widget (criar/destruir/sempre-na-frente) em
        // Windows/Linux, reusando o config ja' lido acima.
        #[cfg(not(target_os = "macos"))]
        if let Some(config) = &config {
            apply_widget(app, config);
        }

        update_tray_menu(app, &snapshot);
    }

    Ok(())
}

fn update_metric(shared: &Arc<SharedState>, metric: UsageMetric) {
    let mut snapshot = shared.snapshot.lock().unwrap();
    match metric.ferramenta.as_str() {
        "codex" => snapshot.codex_metric = Some(metric),
        "claude" => snapshot.claude_metric = Some(metric),
        _ => {}
    }
}

fn mark_success(shared: &Arc<SharedState>) {
    let mut snapshot = shared.snapshot.lock().unwrap();
    snapshot.last_successful_send_at = Some(Utc::now().to_rfc3339());
}

fn clear_last_error(shared: &Arc<SharedState>) {
    let mut snapshot = shared.snapshot.lock().unwrap();
    snapshot.last_error = None;
}

fn handle_runtime_error<R: Runtime>(app: &AppHandle<R>, message: &str) {
    if let Some(shared) = app.try_state::<Arc<SharedState>>() {
        let mut snapshot = shared.snapshot.lock().unwrap();
        snapshot.last_error = Some(message.to_string());
    }

    let _ = append_log_line(
        app.state::<RuntimePaths>().inner(),
        "error",
        message,
        None,
    );

    if let Some(shared) = app.try_state::<Arc<SharedState>>() {
        let _ = refresh_tray(app, &shared);
    }
}

fn build_error_metric(usuario: &str, ferramenta: &str, erro: &str) -> UsageMetric {
    UsageMetric {
        usuario: normalized_user(usuario),
        ferramenta: ferramenta.to_string(),
        uso_percentual: 0.0,
        restante_percentual: 100.0,
        status: "erro".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: None,
        erro: Some(erro.to_string()),
        uso_percentual_7d: None,
        restante_percentual_7d: None,
        reset_em_7d: None,
    }
}

fn normalized_user(usuario: &str) -> String {
    let trimmed = usuario.trim();
    if trimmed.is_empty() {
        "desconhecido".to_string()
    } else {
        trimmed.to_string()
    }
}

fn metric_text(metric: Option<&UsageMetric>) -> String {
    let Some(metric) = metric else {
        return "--".to_string();
    };
    let session = format!("{:.1}%", metric.uso_percentual);
    match metric.uso_percentual_7d {
        Some(seven_day) => format!("{session} | {:.1}% (7d)", seven_day),
        None => session,
    }
}

/// Quais janelas exibir a partir da config "janelas": devolve
/// `(mostra_sessao, mostra_semanal)`. "sessao" -> so 5h; "semanal" -> so 7d;
/// qualquer outro valor (inclusive "ambos") -> as duas.
#[cfg(target_os = "windows")]
fn parse_janelas(value: &str) -> (bool, bool) {
    match value.trim().to_ascii_lowercase().as_str() {
        "sessao" | "sessão" | "session" | "5h" => (true, false),
        "semanal" | "semana" | "weekly" | "7d" => (false, true),
        _ => (true, true),
    }
}

/// Linha de detalhe do widget da barra de tarefas. Com `mostrar_hora = false`
/// usa o tempo restante (`20% (2:36h) | 50% (2d)`); com `true` usa a hora/data
/// exata do reset (`20% (19:20) | 50% (22/06, 19:59)`). `mostra_sessao`/
/// `mostra_semanal` escolhem as janelas (5h/7d); com uma so', o separador "|"
/// some. Se a janela semanal nao tem dados, cai na sessao.
#[cfg(target_os = "windows")]
fn widget_detail(
    metric: Option<&UsageMetric>,
    mostrar_hora: bool,
    mostra_sessao: bool,
    mostra_semanal: bool,
) -> String {
    let Some(metric) = metric else {
        return "--".to_string();
    };
    if metric.status == "erro" {
        return "erro".to_string();
    }

    let suffix = |iso: Option<&str>| {
        if mostrar_hora {
            reset_suffix_clock(iso)
        } else {
            reset_suffix(iso)
        }
    };

    let session = format!("{:.0}%{}", metric.uso_percentual, suffix(metric.reset_em.as_deref()));

    let mut parts: Vec<String> = Vec::new();
    if mostra_sessao {
        parts.push(session.clone());
    }
    if mostra_semanal {
        if let Some(weekly) = metric.uso_percentual_7d {
            parts.push(format!("{:.0}%{}", weekly, suffix(metric.reset_em_7d.as_deref())));
        }
    }
    // Sem nenhuma parte (ex.: so' semanal escolhido mas sem dados de 7d): mostra
    // a sessao para nao ficar vazio.
    if parts.is_empty() {
        return session;
    }
    parts.join(" | ")
}

/// Sufixo " (tempo)" para o reset (tempo restante); vazio quando nao ha reset valido.
#[cfg(target_os = "windows")]
fn reset_suffix(iso: Option<&str>) -> String {
    match format_reset(iso) {
        Some(text) => format!(" ({text})"),
        None => String::new(),
    }
}

/// Sufixo " (hora)" para o reset (hora/data exata); vazio quando nao ha reset valido.
#[cfg(target_os = "windows")]
fn reset_suffix_clock(iso: Option<&str>) -> String {
    match format_reset_clock(iso) {
        Some(text) => format!(" ({text})"),
        None => String::new(),
    }
}

/// Formata a hora/data exata do reset em horario local: "19:20" se for hoje, ou
/// "22/06, 19:59" se for outro dia.
#[cfg(target_os = "windows")]
fn format_reset_clock(iso: Option<&str>) -> Option<String> {
    let reset = DateTime::parse_from_rfc3339(iso?)
        .ok()?
        .with_timezone(&chrono::Local);
    let same_day = reset.date_naive() == chrono::Local::now().date_naive();
    let text = if same_day {
        reset.format("%H:%M").to_string()
    } else {
        reset.format("%d/%m, %H:%M").to_string()
    };
    Some(text)
}

/// Formata o tempo restante ate o reset: "2d", "2:36h" ou "45m".
#[cfg(target_os = "windows")]
fn format_reset(iso: Option<&str>) -> Option<String> {
    let reset = DateTime::parse_from_rfc3339(iso?).ok()?;
    let seconds = (reset.with_timezone(&Utc) - Utc::now()).num_seconds();
    if seconds <= 0 {
        return Some("0m".to_string());
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days >= 1 {
        Some(format!("{days}d"))
    } else if hours >= 1 {
        Some(format!("{hours}:{minutes:02}h"))
    } else {
        Some(format!("{minutes}m"))
    }
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }

    text.chars().take(max_len.saturating_sub(3)).collect::<String>() + "..."
}

fn iso_to_nanos(iso: &str) -> Result<String, String> {
    let timestamp = DateTime::parse_from_rfc3339(iso)
        .map_err(|error| format!("Timestamp invalido para Loki: {error}"))?;
    Ok(timestamp.timestamp_nanos_opt().unwrap_or_default().to_string())
}

fn timestamp_seconds_to_iso(value: i64) -> Option<String> {
    DateTime::from_timestamp(value, 0).map(|timestamp| timestamp.to_rfc3339())
}

fn round_percent(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Percentual restante (100 - usado), arredondado e limitado a 0..=100 — evita
/// exibir valores negativos caso a API devolva uso acima de 100%.
fn remaining_percent(used: f64) -> f64 {
    round_percent((100.0 - used).clamp(0.0, 100.0))
}

fn ensure_storage() -> Result<RuntimePaths, Box<dyn std::error::Error>> {
    let paths = runtime_paths()?;
    fs::create_dir_all(&paths.config_dir)?;
    fs::create_dir_all(&paths.logs_dir)?;
    Ok(paths)
}

fn runtime_paths() -> Result<RuntimePaths, Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    {
        let app_data = env::var("APPDATA")
            .map(PathBuf::from)
            .or_else(|_| dirs::config_dir().ok_or(env::VarError::NotPresent))?;
        let local_app_data = env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|_| dirs::data_local_dir().ok_or(env::VarError::NotPresent))?;

        return Ok(RuntimePaths {
            config_dir: app_data.join(APP_NAME_WINDOWS),
            config_file: app_data.join(APP_NAME_WINDOWS).join("config.json"),
            logs_dir: local_app_data.join(APP_NAME_WINDOWS).join("logs"),
        });
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("Home directory nao encontrada.")?;

        return Ok(RuntimePaths {
            config_dir: home.join(".config").join(APP_NAME_LINUX),
            config_file: home
                .join(".config")
                .join(APP_NAME_LINUX)
                .join("config.json"),
            logs_dir: home.join(".local").join("state").join(APP_NAME_LINUX).join("logs"),
        });
    }

    #[allow(unreachable_code)]
    Err("Sistema operacional nao suportado.".into())
}

fn load_or_create_config(paths: &RuntimePaths) -> Result<AppConfig, Box<dyn std::error::Error>> {
    if !paths.config_file.exists() {
        let default_config = AppConfig::default();
        write_config(paths, &default_config)?;
        return Ok(default_config);
    }

    let content = fs::read_to_string(&paths.config_file)?;
    // Campos ausentes sao preenchidos com os padroes (containers com
    // `#[serde(default)]`), preservando os valores ja existentes no arquivo.
    let mut config: AppConfig = serde_json::from_str(&content)?;
    config.intervalo_segundos = config.intervalo_segundos.clamp(5, 3600);
    config.widget.opacidade = config.widget.opacidade.clamp(0, 100);

    // Normaliza o arquivo: se algo estava faltando (ou fora do clamp), regrava
    // com a estrutura completa. A comparacao e feita sobre `Value` para ignorar
    // diferencas de formatacao/ordem e so reescrever quando houver mudanca real.
    let original: Value = serde_json::from_str(&content)?;
    let canonical: Value = serde_json::to_value(&config)?;
    if original != canonical {
        write_config(paths, &config)?;
    }

    Ok(config)
}

fn write_config(
    paths: &RuntimePaths,
    config: &AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = serde_json::to_string_pretty(config)?;
    fs::write(&paths.config_file, format!("{payload}\n"))?;
    Ok(())
}

fn append_log_line(
    paths: &RuntimePaths,
    level: &str,
    message: &str,
    meta: Option<Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(&paths.logs_dir)?;

    let log_path = paths
        .logs_dir
        .join(format!("{}.log", Utc::now().format("%Y-%m-%d")));
    let mut file = open_append_file(&log_path)?;
    let payload = json!({
        "timestamp": Utc::now().to_rfc3339(),
        "level": level,
        "message": message,
        "meta": meta
    });

    writeln!(file, "{payload}")?;
    Ok(())
}

fn open_append_file(path: &Path) -> Result<File, Box<dyn std::error::Error>> {
    Ok(OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?)
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Abertura de caminho nao suportada neste sistema.".to_string())
}
