use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

mod settings_panel;
mod usage_dashboard;

#[cfg(target_os = "windows")]
mod taskbar_widget;

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
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
}

impl Default for TaskbarConfig {
    fn default() -> Self {
        Self {
            lado: "direita".to_string(),
            deslocamento: 0,
            tamanho_fonte: 9,
            cor_fonte: "auto".to_string(),
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

/// Porta efêmera do servidor HTTP do dashboard de uso (0 = indisponível).
#[derive(Debug, Clone, Copy)]
struct DashboardPort(u16);

/// Porta efêmera do servidor HTTP do painel de configurações (0 = indisponível).
#[derive(Debug, Clone, Copy)]
struct SettingsPort(u16);

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
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            let paths = ensure_storage()?;
            // Garante que o config.json exista e esteja normalizado (campos
            // novos preenchidos com o padrao) ja na inicializacao. A preferencia
            // de exibir na barra de tarefas e lida da config sob demanda.
            load_or_create_config(&paths)?;

            let shared = Arc::new(SharedState {
                snapshot: Mutex::new(RuntimeSnapshot::default()),
                cycle_lock: Mutex::new(()),
                stop: AtomicBool::new(false),
            });

            app.manage(paths.clone());
            app.manage(shared.clone());

            let dashboard_port = match usage_dashboard::start_server() {
                Ok(port) => port,
                Err(error) => {
                    let _ = append_log_line(
                        &paths,
                        "error",
                        "Falha ao iniciar servidor do dashboard de uso.",
                        Some(json!({ "error": error.to_string() })),
                    );
                    0
                }
            };
            app.manage(DashboardPort(dashboard_port));

            let settings_port =
                match settings_panel::start_server(app.handle().clone(), paths.clone()) {
                    Ok(port) => port,
                    Err(error) => {
                        let _ = append_log_line(
                            &paths,
                            "error",
                            "Falha ao iniciar servidor do painel de configuracoes.",
                            Some(json!({ "error": error.to_string() })),
                        );
                        0
                    }
                };
            app.manage(SettingsPort(settings_port));

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
            taskbar_widget::start();

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
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<Arc<SharedState>>() {
                    state.stop.store(true, Ordering::Relaxed);
                }
            }
        });
}

fn create_tray<R: Runtime>(app: &mut tauri::App<R>) -> tauri::Result<()> {
    let (menu, handles) = build_tray_menu(app.handle(), &RuntimeSnapshot::default())?;
    app.manage(handles);

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
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
    let open_settings_item = MenuItem::with_id(
        app,
        "open_settings",
        "Painel de configurações",
        true,
        None::<&str>,
    )?;
    let open_dashboard_item = MenuItem::with_id(
        app,
        "open_dashboard",
        "Dashboard de uso",
        true,
        None::<&str>,
    )?;
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
        &open_settings_item,
        &open_dashboard_item,
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

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, menu_id: &str) {
    match menu_id {
        "open_dashboard" => {
            let port = app
                .try_state::<DashboardPort>()
                .map(|state| state.0)
                .unwrap_or(0);
            if port == 0 {
                handle_runtime_error(app, "Servidor do dashboard de uso nao esta disponivel.");
            } else if let Err(error) = open_url(&format!("http://127.0.0.1:{port}/")) {
                handle_runtime_error(app, &format!("Falha ao abrir dashboard: {error}"));
            }
        }
        "open_settings" => {
            let port = app
                .try_state::<SettingsPort>()
                .map(|state| state.0)
                .unwrap_or(0);
            if port == 0 {
                handle_runtime_error(app, "Servidor do painel de configuracoes nao esta disponivel.");
            } else if let Err(error) = open_url(&format!("http://127.0.0.1:{port}/")) {
                handle_runtime_error(app, &format!("Falha ao abrir painel de configuracoes: {error}"));
            }
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

    let _ = append_log_line(
        &paths,
        "info",
        "Envio manual solicitado pelo usuario.",
        None,
    );

    thread::spawn(move || {
        let _ = run_collection_cycle(&app_handle, &paths, &shared);
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
            let _ = run_collection_cycle(&app, &paths, &shared);
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

fn run_collection_cycle<R: Runtime>(
    app: &AppHandle<R>,
    paths: &RuntimePaths,
    shared: &Arc<SharedState>,
) -> Result<(), String> {
    let _lock = shared.cycle_lock.lock().unwrap();
    let config = load_or_create_config(paths).map_err(|error| error.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;

    let mut had_error = false;

    if config.providers.codex.habilitado {
        match collect_codex_metric(&client, &config) {
            Ok(metric) => {
                update_metric(shared, metric.clone());
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
        restante_percentual: round_percent(100.0 - used_percent),
        status: "ok".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: primary_window.reset_at.and_then(timestamp_seconds_to_iso),
        erro: None,
        uso_percentual_7d: secondary_used_percent.map(round_percent),
        restante_percentual_7d: secondary_used_percent.map(|value| round_percent(100.0 - value)),
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
        restante_percentual: round_percent(100.0 - utilization),
        status: "ok".to_string(),
        coletado_em: Utc::now().to_rfc3339(),
        reset_em: five_hour.resets_at,
        erro: None,
        uso_percentual_7d: seven_day_utilization.map(round_percent),
        restante_percentual_7d: seven_day_utilization.map(|value| round_percent(100.0 - value)),
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
        let snapshot = shared.snapshot.lock().unwrap().clone();
        let tooltip = format!(
            "AiUsageTrayAgent\nCodex: {}\nClaude: {}",
            metric_text(snapshot.codex_metric.as_ref()),
            metric_text(snapshot.claude_metric.as_ref())
        );

        #[cfg(target_os = "windows")]
        {
            tray.set_tooltip(Some(tooltip))?;
            if let Some(paths) = app.try_state::<RuntimePaths>() {
                if let Ok(config) = load_or_create_config(paths.inner()) {
                    taskbar_widget::set_offset(config.barra_tarefas.deslocamento);
                    taskbar_widget::set_side(config.barra_tarefas.lado_esquerdo());
                    taskbar_widget::set_font_size(config.barra_tarefas.tamanho_fonte_pt());
                    taskbar_widget::set_font_color(config.barra_tarefas.cor_fonte_rgb());
                    taskbar_widget::set_provider(
                        "codex",
                        config.providers.codex.habilitado
                            && config.providers.codex.mostra_na_taskbar_windows,
                        widget_detail(snapshot.codex_metric.as_ref()),
                    );
                    taskbar_widget::set_provider(
                        "claude",
                        config.providers.claude.habilitado
                            && config.providers.claude.mostra_na_taskbar_windows,
                        widget_detail(snapshot.claude_metric.as_ref()),
                    );
                }
            }
        }

        #[cfg(target_os = "linux")]
        tray.set_title(Some(format!(
            "C:{} / Cl:{}",
            metric_text(snapshot.codex_metric.as_ref()),
            metric_text(snapshot.claude_metric.as_ref())
        )))?;

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

/// Linha de detalhe do widget da barra de tarefas no formato
/// `20% (2:36h) | 50% (2d)` — uso da sessao (5h) e reset, e uso semanal (7d) e
/// reset. Mostra apenas a parte da sessao quando nao ha dados de 7 dias.
#[cfg(target_os = "windows")]
fn widget_detail(metric: Option<&UsageMetric>) -> String {
    let Some(metric) = metric else {
        return "--".to_string();
    };
    if metric.status == "erro" {
        return "erro".to_string();
    }

    let session = format!(
        "{:.0}%{}",
        metric.uso_percentual,
        reset_suffix(metric.reset_em.as_deref())
    );
    match metric.uso_percentual_7d {
        Some(weekly) => format!(
            "{session} | {:.0}%{}",
            weekly,
            reset_suffix(metric.reset_em_7d.as_deref())
        ),
        None => session,
    }
}

/// Sufixo " (tempo)" para o reset; vazio quando nao ha reset valido.
#[cfg(target_os = "windows")]
fn reset_suffix(iso: Option<&str>) -> String {
    match format_reset(iso) {
        Some(text) => format!(" ({text})"),
        None => String::new(),
    }
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

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Abertura de URL nao suportada neste sistema.".to_string())
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
