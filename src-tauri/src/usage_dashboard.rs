// Dashboard local de uso do Claude Code.
// Replica o painel do Claude Desktop lendo as mesmas fontes:
//   1. ~/.claude/projects/**/*.jsonl  — transcripts vivos (~últimos 30 dias; o cleanup apaga os antigos)
//   2. ~/.claude/stats-cache.json     — baseline histórico consolidado antes do cleanup
// Para bater com o Desktop, a contagem NÃO deduplica linhas repetidas de message.id
// (sessões retomadas copiam linhas pro novo arquivo) — é assim que o app oficial conta.
// Os dados sao expostos ao frontend pelo comando Tauri `get_stats` (IPC); a UI
// vive na webview nativa (sem servidor HTTP/navegador).

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime},
};

use chrono::{DateTime, Local, Timelike};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Default, Serialize)]
struct ModelTotals {
    #[serde(rename = "in")]
    input: u64,
    out: u64,
    #[serde(rename = "cacheRead")]
    cache_read: u64,
    #[serde(rename = "cacheCreate")]
    cache_create: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ProjStats {
    msgs: u64,
    tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct DayStats {
    msgs: u64,
    #[serde(rename = "userMsgs")]
    user_msgs: u64,
    tools: u64,
    sessions: u64,
    models: HashMap<String, ModelTotals>,
    // Contagem de tool_use por nome (Edit, Read, Bash…) e uso por projeto (basename
    // do cwd). Só populados a partir dos transcripts vivos — o baseline não os tem.
    #[serde(rename = "toolsByName")]
    tools_by_name: HashMap<String, u64>,
    projects: HashMap<String, ProjStats>,
}

#[derive(Debug, Clone, Default)]
struct FileAgg {
    days: HashMap<String, DayStats>,
    // (session_id, primeiro timestamp visto neste arquivo)
    sessions: Vec<(String, String)>,
}

struct CacheEntry {
    mtime: SystemTime,
    size: u64,
    agg: FileAgg,
}

static FILE_CACHE: Mutex<Option<HashMap<PathBuf, CacheEntry>>> = Mutex::new(None);

fn claude_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude"))
}

fn local_date(ts: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|value| value.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn parse_transcript(text: &str) -> FileAgg {
    let mut agg = FileAgg::default();
    let mut first_ts_by_session: HashMap<String, String> = HashMap::new();

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let Ok(o) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = o.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "user" && kind != "assistant" {
            continue;
        }
        let Some(ts) = o.get("timestamp").and_then(Value::as_str) else {
            continue;
        };
        let Some(date) = local_date(ts) else {
            continue;
        };

        // Projeto = basename do cwd (ex.: "C:\...\ai-usage-tray-agent" → "ai-usage-tray-agent").
        let proj = o
            .get("cwd")
            .and_then(Value::as_str)
            .and_then(|c| c.rsplit(|ch: char| ch == '\\' || ch == '/').find(|s| !s.is_empty()))
            .map(|s| s.to_string());

        if let Some(session_id) = o.get("sessionId").and_then(Value::as_str) {
            let is_sidechain = o.get("isSidechain").and_then(Value::as_bool).unwrap_or(false);
            if !is_sidechain {
                first_ts_by_session
                    .entry(session_id.to_string())
                    .or_insert_with(|| ts.to_string());
            }
        }

        let message = o.get("message");
        let content = message.and_then(|m| m.get("content")).and_then(Value::as_array);

        if kind == "user" {
            if o.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            // respostas de tool chegam como "user" com content [{type:'tool_result'}] — não são mensagens humanas
            let is_tool_result = content.is_some_and(|items| {
                items
                    .iter()
                    .any(|c| c.get("type").and_then(Value::as_str) == Some("tool_result"))
            });
            if !is_tool_result {
                let day = agg.days.entry(date).or_default();
                day.msgs += 1;
                day.user_msgs += 1;
                if let Some(p) = &proj {
                    day.projects.entry(p.clone()).or_default().msgs += 1;
                }
            }
        } else {
            let day = agg.days.entry(date).or_default();
            day.msgs += 1;
            if let Some(p) = &proj {
                day.projects.entry(p.clone()).or_default().msgs += 1;
            }
            if let Some(items) = content {
                for c in items {
                    if c.get("type").and_then(Value::as_str) == Some("tool_use") {
                        day.tools += 1;
                        if let Some(name) = c.get("name").and_then(Value::as_str) {
                            *day.tools_by_name.entry(name.to_string()).or_default() += 1;
                        }
                    }
                }
            }

            let usage = message.and_then(|m| m.get("usage"));
            let model = message.and_then(|m| m.get("model")).and_then(Value::as_str);
            if let (Some(usage), Some(model)) = (usage, model) {
                if !model.starts_with('<') {
                    let read = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                    let it = read("input_tokens");
                    let ot = read("output_tokens");
                    if let Some(p) = &proj {
                        day.projects.entry(p.clone()).or_default().tokens += it + ot;
                    }
                    let totals = day.models.entry(model.to_string()).or_default();
                    totals.input += it;
                    totals.out += ot;
                    totals.cache_read += read("cache_read_input_tokens");
                    totals.cache_create += read("cache_creation_input_tokens");
                }
            }
        }
    }

    agg.sessions = first_ts_by_session.into_iter().collect();
    agg
}

fn list_jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            list_jsonl_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

struct Baseline {
    last_computed_date: String,
    days: HashMap<String, DayStats>,
    hour_counts: Value,
}

// Baseline do stats-cache.json: cobre o período cujos transcripts já foram apagados pelo cleanup.
fn load_baseline(claude_dir: &Path) -> Option<Baseline> {
    let raw = fs::read_to_string(claude_dir.join("stats-cache.json")).ok()?;
    let cache: Value = serde_json::from_str(&raw).ok()?;
    let last_computed_date = cache.get("lastComputedDate")?.as_str()?.to_string();

    let mut days: HashMap<String, DayStats> = HashMap::new();

    if let Some(activity) = cache.get("dailyActivity").and_then(Value::as_array) {
        for entry in activity {
            let Some(date) = entry.get("date").and_then(Value::as_str) else {
                continue;
            };
            let day = days.entry(date.to_string()).or_default();
            day.msgs = entry.get("messageCount").and_then(Value::as_u64).unwrap_or(0);
            day.tools = entry.get("toolCallCount").and_then(Value::as_u64).unwrap_or(0);
            day.sessions = entry.get("sessionCount").and_then(Value::as_u64).unwrap_or(0);
        }
    }

    // dailyModelTokens traz só o total in+out por modelo/dia; o split in/out vem
    // da proporção global de modelUsage (a soma por modelo fecha exata com o painel)
    let mut ratio_in: HashMap<String, f64> = HashMap::new();
    if let Some(model_usage) = cache.get("modelUsage").and_then(Value::as_object) {
        for (model, usage) in model_usage {
            let input = usage.get("inputTokens").and_then(Value::as_f64).unwrap_or(0.0);
            let output = usage.get("outputTokens").and_then(Value::as_f64).unwrap_or(0.0);
            let total = input + output;
            ratio_in.insert(model.clone(), if total > 0.0 { input / total } else { 0.0 });
        }
    }

    if let Some(daily_tokens) = cache.get("dailyModelTokens").and_then(Value::as_array) {
        for entry in daily_tokens {
            let Some(date) = entry.get("date").and_then(Value::as_str) else {
                continue;
            };
            let Some(by_model) = entry.get("tokensByModel").and_then(Value::as_object) else {
                continue;
            };
            let day = days.entry(date.to_string()).or_default();
            for (model, total) in by_model {
                let total = total.as_f64().unwrap_or(0.0);
                let input = (total * ratio_in.get(model).copied().unwrap_or(0.0)).round();
                day.models.insert(
                    model.clone(),
                    ModelTotals {
                        input: input as u64,
                        out: (total - input).max(0.0) as u64,
                        cache_read: 0,
                        cache_create: 0,
                    },
                );
            }
        }
    }

    Some(Baseline {
        last_computed_date,
        days,
        hour_counts: cache.get("hourCounts").cloned().unwrap_or_else(|| json!({})),
    })
}

fn collect_stats() -> Value {
    let started = Instant::now();
    let Some(claude_dir) = claude_dir() else {
        return json!({ "error": "home dir nao encontrada" });
    };

    let mut files = Vec::new();
    list_jsonl_files(&claude_dir.join("projects"), &mut files);

    let mut reparsed = 0usize;
    {
        let mut guard = FILE_CACHE.lock().unwrap();
        let cache = guard.get_or_insert_with(HashMap::new);

        for file in &files {
            let Ok(meta) = fs::metadata(file) else {
                continue;
            };
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size = meta.len();
            let fresh = cache
                .get(file)
                .is_some_and(|entry| entry.mtime == mtime && entry.size == size);
            if fresh {
                continue;
            }
            let text = fs::read_to_string(file).unwrap_or_default();
            cache.insert(
                file.clone(),
                CacheEntry {
                    mtime,
                    size,
                    agg: parse_transcript(&text),
                },
            );
            reparsed += 1;
        }

        // remove do cache arquivos que sumiram
        cache.retain(|path, _| files.contains(path));
    }

    let baseline = load_baseline(&claude_dir);
    let cutoff = baseline
        .as_ref()
        .map(|value| value.last_computed_date.clone())
        .unwrap_or_default();

    // dias ≤ lastComputedDate vêm do baseline (consolidado); posteriores, dos transcripts
    let mut days: HashMap<String, DayStats> = baseline
        .as_ref()
        .map(|value| value.days.clone())
        .unwrap_or_default();
    let mut session_first_ts: HashMap<String, String> = HashMap::new();

    {
        let guard = FILE_CACHE.lock().unwrap();
        if let Some(cache) = guard.as_ref() {
            for entry in cache.values() {
                for (date, src) in &entry.agg.days {
                    if date.as_str() <= cutoff.as_str() {
                        continue; // já consolidado no baseline
                    }
                    let day = days.entry(date.clone()).or_default();
                    day.msgs += src.msgs;
                    day.user_msgs += src.user_msgs;
                    day.tools += src.tools;
                    for (model, totals) in &src.models {
                        let acc = day.models.entry(model.clone()).or_default();
                        acc.input += totals.input;
                        acc.out += totals.out;
                        acc.cache_read += totals.cache_read;
                        acc.cache_create += totals.cache_create;
                    }
                    for (name, count) in &src.tools_by_name {
                        *day.tools_by_name.entry(name.clone()).or_default() += count;
                    }
                    for (name, proj) in &src.projects {
                        let acc = day.projects.entry(name.clone()).or_default();
                        acc.msgs += proj.msgs;
                        acc.tokens += proj.tokens;
                    }
                }
                for (session_id, ts) in &entry.agg.sessions {
                    session_first_ts
                        .entry(session_id.clone())
                        .and_modify(|current| {
                            if ts < current {
                                *current = ts.clone();
                            }
                        })
                        .or_insert_with(|| ts.clone());
                }
            }
        }
    }

    let mut sessions: Vec<Value> = Vec::new();
    for ts in session_first_ts.values() {
        let Some(date) = local_date(ts) else {
            continue;
        };
        if date.as_str() <= cutoff.as_str() {
            continue; // sessões antigas já estão no baseline
        }
        let hour = DateTime::parse_from_rfc3339(ts)
            .map(|value| value.with_timezone(&Local).hour())
            .unwrap_or(0);
        days.entry(date.clone()).or_default().sessions += 1;
        sessions.push(json!({ "date": date, "hour": hour }));
    }
    // sessões do baseline entram sem hora (a hora agregada vem de hourCounts)
    if let Some(baseline) = &baseline {
        for (date, day) in &baseline.days {
            for _ in 0..day.sessions {
                sessions.push(json!({ "date": date, "hour": Value::Null }));
            }
        }
    }

    let mut days_arr: Vec<Value> = days
        .into_iter()
        .map(|(date, day)| {
            let mut value = serde_json::to_value(&day).unwrap_or_else(|_| json!({}));
            value["date"] = Value::String(date);
            value
        })
        .collect();
    days_arr.sort_by(|a, b| {
        a["date"]
            .as_str()
            .unwrap_or("")
            .cmp(b["date"].as_str().unwrap_or(""))
    });

    json!({
        "generatedAt": Local::now().to_rfc3339(),
        "parseMs": started.elapsed().as_millis() as u64,
        "files": files.len(),
        "reparsed": reparsed,
        "baseline": baseline.as_ref().map(|value| json!({
            "lastComputedDate": value.last_computed_date,
            "hourCounts": value.hour_counts,
        })),
        "days": days_arr,
        "sessions": sessions,
    })
}

/// Comando IPC consumido pela webview do dashboard. Roda como `async` para sair
/// da thread principal; a coleta em si e' sincrona (fs + parse), com cache por
/// arquivo (mtime/size), entao chamadas repetidas sao baratas.
#[tauri::command]
pub async fn get_stats() -> Value {
    collect_stats()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_stats_aggregates_models() {
        let stats = collect_stats();
        let days = stats["days"].as_array().expect("days deve ser array");

        let mut by_model: HashMap<String, (u64, u64)> = HashMap::new();
        for day in days {
            if let Some(models) = day["models"].as_object() {
                for (model, totals) in models {
                    let acc = by_model.entry(model.clone()).or_default();
                    acc.0 += totals["in"].as_u64().unwrap_or(0);
                    acc.1 += totals["out"].as_u64().unwrap_or(0);
                }
            }
        }

        println!("files: {}", stats["files"]);
        println!("sessions: {}", stats["sessions"].as_array().map_or(0, Vec::len));
        let mut models: Vec<_> = by_model.into_iter().collect();
        models.sort_by_key(|(_, (input, out))| std::cmp::Reverse(input + out));
        for (model, (input, out)) in models {
            println!("{model}: {input} in / {out} out");
        }
    }
}
