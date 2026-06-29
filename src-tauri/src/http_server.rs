// Servidor HTTP local (opcional) que serve os dashboards de uso pelo navegador,
// protegido por um PIN. Reaproveita a MESMA SPA da janela nativa: o servidor
// entrega os assets ja' embutidos pelo Tauri (AssetResolver) e expoe um subconjunto
// READ-ONLY dos comandos via `POST /api/invoke/<cmd>`. O frontend (`src/ipc.ts`)
// detecta que nao esta' rodando dentro do Tauri e troca o IPC por `fetch`.
//
// Seguranca (escopo deliberado):
//   - Acesso so' com PIN (tela `/login` -> cookie de sessao em memoria). HTTPS, se
//     desejado, fica a cargo de um proxy externo (ex.: Cloudflare Tunnel) — o
//     servidor fala HTTP puro, adequado a loopback ou atras de um proxy que termina
//     o TLS.
//   - Apenas comandos de LEITURA sao expostos (uso atual, dashboards). Nada de
//     Configuracoes/Envio nem das credenciais (cookie do Claude, token do Codex).
//   - O bind (host/porta), o PIN e o liga/desliga vivem em `config.servidor`.
//
// O servidor (re)inicia ao salvar as Configuracoes (`apply`) e no boot. Trocar
// host/porta exige reinicio do servidor (feito pelo `apply`); o PIN e' relido do
// `config.json` a cada login, entao muda sem reiniciar.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{http_client, read_config, usage_value, RuntimePaths, SharedState};

/// Comandos IPC que o servidor HTTP aceita — apenas leitura dos dashboards. Tudo
/// fora desta lista e' recusado (403), mesmo autenticado.
const COMANDOS_PERMITIDOS: &[&str] = &["get_usage", "force_collect", "get_stats", "get_codex_stats"];

/// Cookie de sessao emitido apos o login com o PIN.
const COOKIE_NOME: &str = "aiusage_sid";

/// Controle do servidor em execucao, para podermos para'-lo e reinicia'-lo quando
/// as Configuracoes mudam (host/porta/PIN/habilitado).
struct ServerHandle {
    stop: Arc<AtomicBool>,
}

/// Servidor ativo (no maximo um). `apply` troca este handle de forma atomica.
fn control() -> &'static Mutex<Option<ServerHandle>> {
    static CONTROL: OnceLock<Mutex<Option<ServerHandle>>> = OnceLock::new();
    CONTROL.get_or_init(|| Mutex::new(None))
}

/// (Re)aplica a configuracao do servidor: para o servidor atual (se houver) e sobe
/// um novo quando `habilitado` e o PIN estiver definido. Chamado no boot e a cada
/// save das Configuracoes. Idempotente.
pub fn apply(app: &AppHandle) {
    // Para o servidor anterior, invalidando as sessoes (o PIN pode ter mudado).
    if let Some(handle) = control().lock().unwrap_or_else(|e| e.into_inner()).take() {
        handle.stop.store(true, Ordering::SeqCst);
    }

    let paths = app.state::<RuntimePaths>().inner().clone();
    let config = read_config(&paths);
    let servidor = config.servidor;

    if !servidor.habilitado {
        return;
    }
    if servidor.pin.trim().is_empty() {
        let _ = crate::append_log_line(
            &paths,
            "error",
            "Servidor HTTP habilitado, mas sem PIN definido — nao iniciado.",
            None,
        );
        return;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let app = app.clone();
    let addr = format!("{}:{}", servidor.host.trim(), servidor.porta);

    thread::spawn(move || run(app, addr, stop_thread));

    *control().lock().unwrap_or_else(|e| e.into_inner()) = Some(ServerHandle { stop });
}

/// Loop principal do servidor: faz o bind e atende as requisicoes ate' o flag de
/// parada ser acionado (no `apply` seguinte). `recv_timeout` permite checar o flag
/// periodicamente sem bloquear para sempre.
fn run(app: AppHandle, addr: String, stop: Arc<AtomicBool>) {
    let paths = app.state::<RuntimePaths>().inner().clone();

    let server = match Server::http(&addr) {
        Ok(server) => server,
        Err(error) => {
            let _ = crate::append_log_line(
                &paths,
                "error",
                &format!("Falha ao iniciar o servidor HTTP em {addr}: {error}"),
                None,
            );
            return;
        }
    };
    let _ = crate::append_log_line(
        &paths,
        "info",
        &format!("Servidor HTTP dos dashboards iniciado em http://{addr}"),
        None,
    );

    // Sessoes validas (tokens dos cookies). Vivem enquanto este servidor roda; um
    // reinicio (troca de PIN/porta) zera tudo, forcando novo login.
    let sessions: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(500)) {
            Ok(Some(request)) => handle_request(&app, &paths, &sessions, request),
            Ok(None) => continue, // timeout — re-checa o flag de parada
            Err(_) => break,
        }
    }
}

fn handle_request(
    app: &AppHandle,
    paths: &RuntimePaths,
    sessions: &Arc<Mutex<HashSet<String>>>,
    request: Request,
) {
    let method = request.method().clone();
    let raw_url = request.url().to_string();
    let path = raw_url.split('?').next().unwrap_or("/").to_string();

    match (&method, path.as_str()) {
        (&Method::Get, "/favicon.ico") => serve_favicon(request),
        (&Method::Post, "/api/login") => handle_login(paths, sessions, request),
        (&Method::Post, "/api/logout") => handle_logout(sessions, request),
        (&Method::Get, "/login") => {
            let erro = raw_url.split('?').nth(1).is_some_and(|q| q.contains("erro=1"));
            respond_html(request, 200, login_page(erro));
        }
        (&Method::Post, p) if p.starts_with("/api/invoke/") => {
            if !is_authed(sessions, &request) {
                return respond_json(request, 401, json!({ "error": "nao autenticado" }));
            }
            let cmd = p.trim_start_matches("/api/invoke/").to_string();
            handle_invoke(app, paths, request, &cmd);
        }
        (&Method::Get, _) => {
            // Navegacao HTML (raiz/qualquer .html) exige sessao; assets estaticos
            // (js/css/imagens) sao publicos — nao expoem dados sem o /api/invoke.
            let html_nav = path == "/" || path.ends_with(".html");
            if html_nav && !is_authed(sessions, &request) {
                return redirect(request, "/login");
            }
            serve_asset(app, &path, request);
        }
        _ => respond_text(request, 404, "not found"),
    }
}

/// Valida o PIN (relido do `config.json` a cada tentativa) e, em caso de sucesso,
/// emite um cookie de sessao e redireciona para a raiz. Em falha, volta para a tela
/// de login com um pequeno atraso (mitiga tentativa por forca bruta).
fn handle_login(
    paths: &RuntimePaths,
    sessions: &Arc<Mutex<HashSet<String>>>,
    mut request: Request,
) {
    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    let pin_informado = extrair_pin(&body);
    let pin_config = read_config(paths).servidor.pin.trim().to_string();

    if pin_config.is_empty() || pin_informado != pin_config {
        thread::sleep(Duration::from_millis(700));
        return redirect(request, "/login?erro=1");
    }

    let token = gerar_token();
    sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token.clone());

    let set_cookie = format!("{COOKIE_NOME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000");
    let response = Response::empty(302)
        .with_header(header("Location", "/"))
        .with_header(header("Set-Cookie", &set_cookie));
    let _ = request.respond(response);
}

fn handle_logout(sessions: &Arc<Mutex<HashSet<String>>>, request: Request) {
    if let Some(token) = cookie_token(&request) {
        sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&token);
    }
    let expira = format!("{COOKIE_NOME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    let response = Response::empty(302)
        .with_header(header("Location", "/login"))
        .with_header(header("Set-Cookie", &expira));
    let _ = request.respond(response);
}

/// Despacha um comando permitido para a mesma logica da janela nativa, reusando os
/// helpers do `lib.rs`. Recusa qualquer comando fora da allowlist.
fn handle_invoke(app: &AppHandle, paths: &RuntimePaths, mut request: Request, cmd: &str) {
    if !COMANDOS_PERMITIDOS.contains(&cmd) {
        return respond_json(request, 403, json!({ "error": "comando nao permitido" }));
    }

    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    let args: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));

    let shared = app.state::<Arc<SharedState>>().inner().clone();

    let resultado: Value = match cmd {
        "get_usage" => usage_value(paths, &shared),
        "force_collect" => {
            // "Atualizar agora": forca uma coleta nova (respeitando as regras de
            // envio, igual ao comando nativo) e devolve o uso ja' atualizado.
            let _ = crate::run_collection_cycle(app, paths, &shared, false);
            usage_value(paths, &shared)
        }
        "get_stats" => crate::usage_dashboard::collect_stats(),
        "get_codex_stats" => {
            let days = args.get("days").and_then(Value::as_u64).unwrap_or(30) as u32;
            let start = args.get("start").and_then(Value::as_str).map(str::to_string);
            let end = args.get("end").and_then(Value::as_str).map(str::to_string);
            let config = read_config(paths);
            crate::codex_dashboard::collect(
                &http_client(),
                &config.providers.codex.auth_json_path,
                days,
                start,
                end,
            )
        }
        _ => json!({ "error": "comando nao permitido" }),
    };

    respond_json(request, 200, resultado);
}

/// Serve um asset embutido (a SPA buildada). `/` cai em `/index.html`. Em `tauri
/// dev` os assets nao ficam embutidos, entao pode retornar 404 — o servidor e'
/// pensado para o app empacotado (ou atras de um proxy).
fn serve_asset(app: &AppHandle, path: &str, request: Request) {
    let alvo = if path == "/" { "/index.html" } else { path };
    let resolver = app.asset_resolver();
    let asset = resolver
        .get(alvo.to_string())
        .or_else(|| resolver.get(alvo.trim_start_matches('/').to_string()));

    match asset {
        Some(asset) => {
            let response = Response::from_data(asset.bytes)
                .with_header(header("Content-Type", &asset.mime_type));
            let _ = request.respond(response);
        }
        None => respond_text(request, 404, "asset nao encontrado"),
    }
}

/// Bytes do favicon (o mesmo icone do app), embutidos no binario.
const FAVICON_ICO: &[u8] = include_bytes!("../icons/icon.ico");

/// Serve o favicon. Publico — o navegador o requisita automaticamente, sem
/// cookie, tanto na tela de login quanto na SPA. Evita o 404 que deixava a aba
/// sem icone.
fn serve_favicon(request: Request) {
    let response = Response::from_data(FAVICON_ICO.to_vec())
        .with_header(header("Content-Type", "image/x-icon"))
        .with_header(header("Cache-Control", "public, max-age=86400"));
    let _ = request.respond(response);
}

// ---- Sessao / autenticacao -------------------------------------------------

fn is_authed(sessions: &Arc<Mutex<HashSet<String>>>, request: &Request) -> bool {
    match cookie_token(request) {
        Some(token) => sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&token),
        None => false,
    }
}

/// Extrai o token do cookie de sessao do cabecalho `Cookie`, se presente.
fn cookie_token(request: &Request) -> Option<String> {
    let header = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Cookie"))?;
    let valor = header.value.as_str();
    for parte in valor.split(';') {
        let parte = parte.trim();
        if let Some(token) = parte.strip_prefix(&format!("{COOKIE_NOME}=")) {
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Token de sessao opaco (256 bits aleatorios em base64 url-safe).
fn gerar_token() -> String {
    let mut buf = [0u8; 32];
    // Em caso (improvavel) de falha do RNG do SO, o token fica previsivel — mas o
    // proprio login exige o PIN, entao a falha nao concede acesso por si so'.
    let _ = getrandom::getrandom(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Le o campo `pin` de um corpo `application/x-www-form-urlencoded`
/// (`pin=1234&...`), aplicando percent-decoding e `+` -> espaco.
fn extrair_pin(body: &str) -> String {
    for par in body.split('&') {
        let mut it = par.splitn(2, '=');
        let chave = it.next().unwrap_or("");
        if chave == "pin" {
            return url_decode(it.next().unwrap_or(""));
        }
    }
    String::new()
}

/// Decodificador minimo de `x-www-form-urlencoded`: `+` vira espaco e `%XX` vira o
/// byte correspondente. Bytes invalidos sao mantidos como estao.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).trim().to_string()
}

// ---- Helpers de resposta ---------------------------------------------------

fn header(field: &str, value: &str) -> Header {
    // `from_bytes` so' falha com bytes invalidos em cabecalho; os usos aqui sao
    // todos literais validos, entao o unwrap nunca dispara.
    Header::from_bytes(field.as_bytes(), value.as_bytes())
        .unwrap_or_else(|_| Header::from_bytes(&b"X-Invalid"[..], &b"1"[..]).unwrap())
}

fn respond_json(request: Request, status: u16, value: Value) {
    let response = Response::from_string(value.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"));
    let _ = request.respond(response);
}

fn respond_html(request: Request, status: u16, html: String) {
    let response = Response::from_string(html)
        .with_status_code(status)
        .with_header(header("Content-Type", "text/html; charset=utf-8"));
    let _ = request.respond(response);
}

fn respond_text(request: Request, status: u16, text: &str) {
    let response = Response::from_string(text).with_status_code(status);
    let _ = request.respond(response);
}

fn redirect(request: Request, location: &str) {
    let response = Response::empty(302).with_header(header("Location", location));
    let _ = request.respond(response);
}

/// Pagina de login (HTML puro, sem JavaScript): um formulario que posta o PIN para
/// `/api/login`. O aviso de erro e' renderizado no servidor (sem script), entao a
/// pagina funciona mesmo sob CSP restritiva. `erro = true` mostra o aviso de PIN
/// incorreto.
fn login_page(erro: bool) -> String {
    let aviso = if erro {
        r#"<div class="erro">PIN incorreto. Tente novamente.</div>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AiUsage — acesso</title>
<style>
  :root {{ color-scheme: dark; }}
  * {{ box-sizing: border-box; }}
  body {{ margin: 0; min-height: 100vh; display: flex; align-items: center; justify-content: center;
    font: 15px/1.4 "Segoe UI", system-ui, sans-serif;
    background: #1a1915; color: #e8e6e1; }}
  .card {{ width: min(360px, 92vw); background: #232220; border: 1px solid #34322d;
    border-radius: 14px; padding: 28px 26px; box-shadow: 0 16px 40px rgba(0,0,0,.5); }}
  .brand {{ font-size: 17px; font-weight: 600; color: #d97757; margin-bottom: 16px; }}
  h1 {{ margin: 0 0 4px; font-size: 20px; font-weight: 600; }}
  p {{ margin: 0 0 20px; color: #9a968d; font-size: 13px; }}
  label {{ display: block; font-size: 14px; color: #c9c5bc; margin-bottom: 6px; }}
  input {{ width: 100%; padding: 9px 11px; font: inherit; font-size: 15px; border-radius: 9px;
    border: 1px solid #3a3833; background: #1c1b18; color: #e8e6e1; outline: none; }}
  input:focus {{ border-color: #d97757; }}
  button {{ width: 100%; margin-top: 18px; padding: 10px; font: inherit; font-size: 14px; font-weight: 600;
    border: 1px solid #d97757; border-radius: 9px; background: #d97757; color: #1a1915; cursor: pointer; }}
  button:hover {{ background: #e08a6d; }}
  .erro {{ margin: 0 0 14px; padding: 9px 11px; border-radius: 9px; font-size: 13px;
    background: #2c211d; border: 1px solid #5a3a30; color: #e0a890; }}
</style>
</head>
<body>
  <form class="card" method="POST" action="/api/login" autocomplete="off">
    <div class="brand">AiUsage</div>
    <h1>Dashboards de uso</h1>
    <p>Informe o PIN de acesso.</p>
    {aviso}
    <label for="pin">PIN</label>
    <input type="password" id="pin" name="pin" inputmode="numeric" autofocus required>
    <button type="submit">Entrar</button>
  </form>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sobe um servidor tiny_http real numa porta efemera roteando so'
    /// `/favicon.ico` (mesmo caminho do roteador de producao) e faz um GET HTTP
    /// de verdade, validando status, content-type e os bytes do icone. Prova que a
    /// rota do favicon serve o icone do app — nao so' que compila.
    #[test]
    fn favicon_serve_icone_via_http() {
        let server = Server::http("127.0.0.1:0").expect("bind efemero");
        let port = server
            .server_addr()
            .to_ip()
            .expect("addr ip")
            .port();

        let handle = thread::spawn(move || {
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(5)) {
                // Mesmo despacho do roteador real para esse caminho.
                if request.url() == "/favicon.ico" {
                    serve_favicon(request);
                }
            }
        });

        let response = reqwest::blocking::get(format!("http://127.0.0.1:{port}/favicon.ico"))
            .expect("GET /favicon.ico");

        assert!(response.status().is_success(), "status {}", response.status());
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert_eq!(content_type, "image/x-icon");

        let bytes = response.bytes().expect("corpo");
        assert!(!bytes.is_empty(), "favicon vazio");
        assert_eq!(bytes.as_ref(), FAVICON_ICO, "bytes do favicon batem com o icone");

        handle.join().expect("thread do servidor");
    }

    /// O favicon embutido nao e' vazio e tem a assinatura de um arquivo .ico
    /// (cabecalho ICONDIR: reservado=0, tipo=1).
    #[test]
    fn favicon_embutido_e_um_ico_valido() {
        assert!(FAVICON_ICO.len() > 4, "icone curto demais");
        assert_eq!(&FAVICON_ICO[0..4], &[0x00, 0x00, 0x01, 0x00], "cabecalho .ico");
    }

    /// Garante o parsing do PIN no corpo do POST de login (form-urlencoded),
    /// incluindo percent-decoding — o caminho que valida o acesso.
    #[test]
    fn extrai_pin_do_corpo_do_login() {
        assert_eq!(extrair_pin("pin=1234"), "1234");
        assert_eq!(extrair_pin("pin=12%2034"), "12 34"); // %20 -> espaco
        assert_eq!(extrair_pin("outro=x&pin=ab9"), "ab9");
        assert_eq!(extrair_pin("semcampo=1"), "");
    }
}
