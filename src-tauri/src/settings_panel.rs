//! Painel de configuracoes servido localmente (mesma abordagem do dashboard de
//! uso): um servidor HTTP minimal em 127.0.0.1:0 que serve uma pagina de
//! formulario e expoe `GET/POST /api/settings` para ler/gravar o `config.json`
//! e a preferencia de "iniciar com o sistema" (autostart, gerenciado pelo
//! plugin, fora do config.json).
//!
//! A pagina e' aberta no navegador pelo item "Painel de configuracoes" do tray.
//! Como o servidor escuta so' em loopback, o modelo de confianca e' o mesmo do
//! dashboard. Para o POST (que grava em disco) exigimos `Content-Type:
//! application/json`, o que ja' bloqueia POST cross-origin de um navegador (que
//! precisaria de preflight CORS, nao respondido aqui).

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::RuntimePaths;

const SETTINGS_HTML: &str = include_str!("../assets/settings-panel.html");

/// Corpo aceito no POST: o `config.json` completo mais a preferencia de
/// autostart (que nao mora no config.json).
#[derive(Deserialize)]
struct SaveRequest {
    config: crate::AppConfig,
    #[serde(default)]
    autostart: bool,
}

/// Inicia o servidor do painel em uma porta efemera (127.0.0.1:0) e retorna a
/// porta sorteada pelo sistema operacional.
pub fn start_server<R: Runtime>(app: AppHandle<R>, paths: RuntimePaths) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            let paths = paths.clone();
            thread::spawn(move || handle_connection(stream, &app, &paths));
        }
    });

    Ok(port)
}

fn handle_connection<R: Runtime>(mut stream: TcpStream, app: &AppHandle<R>, paths: &RuntimePaths) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let route = request.path.split('?').next().unwrap_or("/");

    match (request.method.as_str(), route) {
        ("GET", "/") | ("GET", "/index.html") => {
            respond(&mut stream, "200 OK", "text/html; charset=utf-8", SETTINGS_HTML);
        }
        ("GET", "/api/settings") => {
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &settings_json(app, paths).to_string(),
            );
        }
        ("POST", "/api/settings") => {
            // CSRF-lite: exigir application/json bloqueia POST cross-origin de
            // navegador (que dispararia preflight CORS, nao respondido aqui).
            if !request
                .content_type
                .to_ascii_lowercase()
                .contains("application/json")
            {
                respond(
                    &mut stream,
                    "415 Unsupported Media Type",
                    "application/json; charset=utf-8",
                    &json!({ "error": "content-type deve ser application/json" }).to_string(),
                );
                return;
            }
            match save_settings(app, paths, &request.body) {
                Ok(value) => respond(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &value.to_string(),
                ),
                Err(error) => respond(
                    &mut stream,
                    "400 Bad Request",
                    "application/json; charset=utf-8",
                    &json!({ "error": error }).to_string(),
                ),
            }
        }
        _ => respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found",
        ),
    }
}

/// Estado atual exposto ao painel: o `config.json` (normalizado), a preferencia
/// de autostart e o SO (para rotulos e campos especificos do Windows).
fn settings_json<R: Runtime>(app: &AppHandle<R>, paths: &RuntimePaths) -> Value {
    let config = crate::load_or_create_config(paths).unwrap_or_default();
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);

    let mut value = json!({
        "autostart": autostart,
        "os": std::env::consts::OS,
        "autostartLabel": autostart_label(),
    });
    value["config"] = serde_json::to_value(&config).unwrap_or(Value::Null);
    value
}

fn save_settings<R: Runtime>(
    app: &AppHandle<R>,
    paths: &RuntimePaths,
    body: &str,
) -> Result<Value, String> {
    let request: SaveRequest =
        serde_json::from_str(body).map_err(|error| format!("JSON invalido: {error}"))?;

    // Grava o config; a normalizacao (clamp de intervalo/fonte, validacao de cor)
    // acontece na releitura logo abaixo, refletida na resposta. O worker tambem
    // detecta a mudanca pelo mtime e aplica tudo (barra/tray) em ~1s.
    crate::write_config(paths, &request.config)
        .map_err(|error| format!("falha ao salvar config.json: {error}"))?;

    apply_autostart(app, request.autostart);

    Ok(settings_json(app, paths))
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

fn autostart_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Iniciar com o Windows"
    } else {
        "Iniciar com o sistema"
    }
}

struct Request {
    method: String,
    path: String,
    content_type: String,
    body: String,
}

/// Le uma requisicao HTTP completa: linha de status, headers e (se houver) o
/// corpo conforme `Content-Length`. Retorna `None` em erro de I/O.
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    loop {
        if let Some(header_end) = find_subsequence(&buffer, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let content_length = header_value(&headers, "content-length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let body_start = header_end + 4;

            while buffer.len() < body_start + content_length {
                let read = stream.read(&mut chunk).ok()?;
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }

            let first_line = headers.lines().next().unwrap_or("");
            let mut parts = first_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("/").to_string();
            let content_type = header_value(&headers, "content-type").unwrap_or_default();
            let end = (body_start + content_length).min(buffer.len());
            let body = String::from_utf8_lossy(&buffer[body_start..end]).to_string();

            return Some(Request {
                method,
                path,
                content_type,
                body,
            });
        }

        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        // Guarda contra requisicoes absurdas (config.json e' pequeno).
        if buffer.len() > 1_000_000 {
            return None;
        }
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Valor de um header pelo nome (case-insensitive), ignorando a linha de status.
fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case(name) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}