//! Optional assistant: the person's own API key, a short description of the video (never the video or the sound),
//! and a sentence like "cut the first ten seconds". The answer is a list of small edit steps the editor can apply.
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Facts {
    pub duration: f64,
    #[serde(default)]
    pub markers: Vec<f64>,
    #[serde(default)]
    pub silences: Vec<(f64, f64)>,
    #[serde(default)]
    pub idle: Vec<(f64, f64)>,
    #[serde(default)]
    pub scenes: Vec<f64>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Op {
    pub op: String,
    pub from: f64,
    pub to: f64,
    pub speed: f64,
    pub text: String,
}

#[derive(Serialize, Debug)]
pub struct Reply {
    pub explain: String,
    pub ops: Vec<Op>,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

pub const SYSTEM: &str = "You are the editing assistant inside a screen recorder. You get a short description of one video (seconds) and an instruction. \
Answer with ONE JSON object and nothing else: {\"explain\": \"one short sentence in the language of the instruction\", \"ops\": [...]}. \
Allowed ops (all times are seconds of the ORIGINAL video): \
{\"op\":\"cut\",\"from\":a,\"to\":b} removes that part; \
{\"op\":\"keep\",\"from\":a,\"to\":b} keeps only that part and removes the rest; \
{\"op\":\"speed\",\"from\":a,\"to\":b,\"speed\":2} changes the speed (0.5 to 4); \
{\"op\":\"split\",\"from\":a} splits at a; \
{\"op\":\"text\",\"from\":a,\"to\":b,\"text\":\"...\"} shows a caption. \
Use the given silences, idle (still screen) and scenes when they help. Never invent times outside the video. Do not add anything the person did not ask for. If the instruction cannot be done with these ops, return an empty ops list and explain why.";

fn round(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn list(v: &[(f64, f64)]) -> String {
    v.iter().take(40).map(|(a, b)| format!("{}-{}", round(*a), round(*b))).collect::<Vec<_>>().join(", ")
}

/// The compact description sent with the instruction.
pub fn describe(f: &Facts) -> String {
    let mut s = format!("Video length: {}s.\n", round(f.duration));
    if !f.markers.is_empty() {
        s += &format!("Markers the person pressed while recording: {}.\n", f.markers.iter().take(40).map(|m| round(*m).to_string()).collect::<Vec<_>>().join(", "));
    }
    if !f.silences.is_empty() {
        s += &format!("Silent parts: {}.\n", list(&f.silences));
    }
    if !f.idle.is_empty() {
        s += &format!("Still-screen parts: {}.\n", list(&f.idle));
    }
    if !f.scenes.is_empty() {
        s += &format!("Scene changes at: {}.\n", f.scenes.iter().take(40).map(|m| round(*m).to_string()).collect::<Vec<_>>().join(", "));
    }
    s
}

/// A rough size of the request, so the person can see what it costs before sending. Arabic and other scripts use more tokens.
pub fn estimate_tokens(instruction: &str, f: &Facts) -> u64 {
    let chars = (SYSTEM.chars().count() + describe(f).chars().count() + instruction.chars().count()) as f64;
    let non_latin = instruction.chars().filter(|c| !c.is_ascii()).count() as f64;
    (chars / 3.6 + non_latin * 0.6).round() as u64
}

fn first_object(text: &str) -> Option<&str> {
    let a = text.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in text[a..].char_indices() {
        if in_str {
            if esc { esc = false } else if c == '\\' { esc = true } else if c == '"' { in_str = false }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[a..a + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Reads the model's answer and keeps only steps that make sense for a video of this length.
pub fn parse(text: &str, duration: f64) -> Result<(String, Vec<Op>)> {
    let obj = first_object(text).ok_or_else(|| anyhow!("The assistant did not answer with edit steps"))?;
    let v: Value = serde_json::from_str(obj).map_err(|e| anyhow!("The assistant's answer could not be read: {e}"))?;
    let explain = v["explain"].as_str().unwrap_or("").to_string();
    let mut ops = vec![];
    for o in v["ops"].as_array().cloned().unwrap_or_default() {
        let kind = o["op"].as_str().unwrap_or("").to_string();
        let a = o["from"].as_f64().unwrap_or(-1.0);
        let b = o["to"].as_f64().unwrap_or(a);
        if a < 0.0 || a > duration {
            continue;
        }
        let (from, to) = (a.min(duration), b.clamp(a, duration));
        match kind.as_str() {
            "cut" | "keep" if to - from >= 0.2 => ops.push(Op { op: kind, from, to, speed: 1.0, text: String::new() }),
            "speed" if to - from >= 0.2 => ops.push(Op { op: kind, from, to, speed: o["speed"].as_f64().unwrap_or(1.0).clamp(0.5, 4.0), text: String::new() }),
            "split" => ops.push(Op { op: kind, from, to: from, speed: 1.0, text: String::new() }),
            "text" if !o["text"].as_str().unwrap_or("").trim().is_empty() => ops.push(Op { op: kind, from, to: if to - from < 0.5 { (from + 3.0).min(duration) } else { to }, speed: 1.0, text: o["text"].as_str().unwrap_or("").trim().chars().take(200).collect() }),
            _ => {}
        }
        if ops.len() >= 60 {
            break;
        }
    }
    Ok((explain, ops))
}

/// How to reach one provider. Everything except Claude speaks the same chat format, so adding a provider is one more row here.
pub struct Prov {
    pub id: &'static str,
    pub url: &'static str,
    pub model: &'static str,
    /// whether the provider accepts the "answer as JSON" switch
    pub json_mode: bool,
}

pub const PROVIDERS: &[Prov] = &[
    Prov { id: "anthropic", url: "https://api.anthropic.com/v1/messages", model: "claude-haiku-5-5", json_mode: false },
    Prov { id: "openai", url: "https://api.openai.com/v1/chat/completions", model: "gpt-4o-mini", json_mode: true },
    Prov { id: "gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions", model: "gemini-2.5-flash", json_mode: false },
    Prov { id: "openrouter", url: "https://openrouter.ai/api/v1/chat/completions", model: "openai/gpt-4o-mini", json_mode: false },
    Prov { id: "deepseek", url: "https://api.deepseek.com/chat/completions", model: "deepseek-flash", json_mode: true },
    Prov { id: "mistral", url: "https://api.mistral.ai/v1/chat/completions", model: "mistral-small-latest", json_mode: true },
    Prov { id: "groq", url: "https://api.groq.com/openai/v1/chat/completions", model: "llama-3.3-70b-versatile", json_mode: true },
    Prov { id: "xai", url: "https://api.x.ai/v1/chat/completions", model: "grok-3-mini", json_mode: false },
    // any service that speaks the same chat format, including a model running on this computer (Ollama, LM Studio)
    Prov { id: "custom", url: "", model: "", json_mode: false },
];

fn label(id: &str) -> &'static str {
    match id {
        "anthropic" => "Claude",
        "openai" => "OpenAI",
        "gemini" => "Google Gemini",
        "openrouter" => "OpenRouter",
        "deepseek" => "DeepSeek",
        "mistral" => "Mistral",
        "groq" => "Groq",
        "xai" => "xAI",
        _ => "The service",
    }
}

fn prov(id: &str) -> &'static Prov {
    PROVIDERS.iter().find(|p| p.id == id).unwrap_or(&PROVIDERS[0])
}

pub fn default_model(provider: &str) -> &'static str {
    prov(provider).model
}

/// The full address of a custom provider: accepts a base such as http://localhost:11434/v1 or the complete chat address.
pub fn custom_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/chat/completions") { b.to_string() } else { format!("{b}/chat/completions") }
}

pub struct Cfg<'a> {
    pub provider: &'a str,
    pub key: &'a str,
    pub model: &'a str,
    pub base: &'a str,
}

/// A request ready to send: address, headers and body. Built separately from sending so every provider's format can be tested.
pub struct Req {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

fn endpoint(c: &Cfg, path: &str) -> Result<String> {
    if c.provider == "custom" {
        let b = c.base.trim().trim_end_matches('/');
        if !(b.starts_with("http://") || b.starts_with("https://")) {
            return Err(anyhow!("Enter the address of the service first, for example http://localhost:11434/v1"));
        }
        let root = b.trim_end_matches("/chat/completions").trim_end_matches("/models");
        return Ok(format!("{root}/{path}"));
    }
    let chat = prov(c.provider).url;
    Ok(if path == "models" { chat.replace("/chat/completions", "/models").replace("/v1/messages", "/v1/models") } else { chat.to_string() })
}

fn auth(c: &Cfg) -> Vec<(String, String)> {
    if c.provider == "anthropic" {
        vec![("x-api-key".into(), c.key.into()), ("anthropic-version".into(), "2023-06-01".into())]
    } else if c.key.is_empty() {
        vec![]
    } else {
        vec![("Authorization".into(), format!("Bearer {}", c.key))]
    }
}

pub fn chat_request(c: &Cfg, system: &str, user: &str, max_tokens: u32) -> Result<Req> {
    let p = prov(c.provider);
    if c.provider == "anthropic" {
        return Ok(Req { url: endpoint(c, "chat")?, headers: auth(c), body: json!({"model": c.model, "max_tokens": max_tokens, "system": system, "messages": [{"role": "user", "content": user}]}) });
    }
    // OpenAI's newer models refuse "max_tokens"; everyone else still expects it
    let limit = if c.provider == "openai" { "max_completion_tokens" } else { "max_tokens" };
    let mut body = json!({"model": c.model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]});
    body[limit] = json!(max_tokens);
    if p.json_mode {
        body["response_format"] = json!({"type": "json_object"});
    }
    Ok(Req { url: endpoint(c, "chat/completions")?, headers: auth(c), body })
}

pub fn models_request(c: &Cfg) -> Result<Req> {
    Ok(Req { url: endpoint(c, "models")?, headers: auth(c), body: Value::Null })
}

/// Chat models from a provider's model list, without embeddings, speech, images and the like.
pub fn parse_models(v: &Value) -> Vec<String> {
    let arr = v["data"].as_array().or_else(|| v["models"].as_array()).cloned().unwrap_or_default();
    let skip = ["embed", "whisper", "tts", "transcribe", "moderation", "image", "dall-e", "audio", "realtime", "rerank", "vision-exp", "guard", "imagen", "veo", "aqa", "speech", "ocr", "search-preview", "computer-use"];
    let mut out: Vec<String> = arr
        .iter()
        .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()))
        .map(|s| s.trim_start_matches("models/").to_string())
        .filter(|s| !s.is_empty() && !skip.iter().any(|k| s.to_lowercase().contains(k)))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The most sensible low-cost model from a live list, or None when nothing fits and the caller should keep its default.
pub fn pick_default(provider: &str, ids: &[String]) -> Option<String> {
    let has = |s: &str| ids.iter().find(|i| i.as_str() == s).cloned();
    let newest = |pred: &dyn Fn(&str) -> bool| ids.iter().filter(|i| pred(&i.to_lowercase())).max().cloned();
    let plain = |i: &str| !["lite", "preview", "exp", "thinking", "latest-", "-001", "-0"].iter().any(|k| i.contains(k));
    match provider {
        "anthropic" => newest(&|i| i.contains("haiku")),
        "openai" => has("gpt-4o-mini").or_else(|| newest(&|i| i.contains("mini") && i.starts_with("gpt") && plain(i))),
        "gemini" => newest(&|i| i.contains("flash") && !i.contains("lite") && !i.contains("preview") && !i.contains("exp") && !i.contains("live") && !i.contains("image")),
        "deepseek" => has("deepseek-flash").or_else(|| newest(&|i| i.contains("flash"))).or_else(|| has("deepseek-chat")),
        "mistral" => has("mistral-small-latest").or_else(|| newest(&|i| i.contains("small"))),
        "groq" => has("llama-3.3-70b-versatile").or_else(|| newest(&|i| i.contains("llama") && i.contains("70b"))),
        "xai" => newest(&|i| i.contains("mini")),
        "openrouter" => has("openai/gpt-4o-mini"),
        "custom" => ids.first().cloned(),
        _ => None,
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(90)).build()
}

fn explain(c: &Cfg, code: u16, body: &Value) -> anyhow::Error {
    let who = label(c.provider);
    let msg = body["error"]["message"].as_str().or_else(|| body["error"].as_str()).or_else(|| body[0]["error"]["message"].as_str()).unwrap_or("").to_string();
    anyhow!(match code {
        401 | 403 => format!("{who} did not accept the key. Check that it is the key for {who} and that you copied all of it."),
        402 | 429 => format!("{who} refused the request (limit or no credit). {msg}"),
        404 => format!("That model name or address is not available on {who}. {msg}"),
        _ => format!("{who} returned an error ({code}). {msg}"),
    })
}

/// The models this key can use right now, with the one Recora suggests.
pub fn list_models(c: &Cfg) -> Result<(Vec<String>, Option<String>)> {
    if c.key.trim().is_empty() && c.provider != "custom" && c.provider != "openrouter" {
        return Err(anyhow!("Add your API key first"));
    }
    let r = models_request(c)?;
    let mut req = agent().get(&r.url);
    for (k, v) in &r.headers {
        req = req.set(k, v);
    }
    let v: Value = match req.call() {
        Ok(r) => r.into_json()?,
        Err(ureq::Error::Status(code, r)) => return Err(explain(c, code, &r.into_json().unwrap_or(Value::Null))),
        Err(e) => return Err(anyhow!("Could not reach {}. Check the internet connection. ({e})", label(c.provider))),
    };
    let ids = parse_models(&v);
    if ids.is_empty() {
        return Err(anyhow!("{} returned no models for this key.", label(c.provider)));
    }
    let pick = pick_default(c.provider, &ids);
    Ok((ids, pick))
}

fn call(c: &Cfg, system: &str, user: &str, max_tokens: u32) -> Result<(String, u64, u64)> {
    let provider = c.provider;
    let r = chat_request(c, system, user, max_tokens)?;
    let mut req = agent().post(&r.url);
    for (k, v) in &r.headers {
        req = req.set(k, v);
    }
    let resp = req.send_json(r.body);
    let v: Value = match resp {
        Ok(r) => r.into_json()?,
        Err(ureq::Error::Status(code, r)) => return Err(explain(c, code, &r.into_json().unwrap_or(Value::Null))),
        Err(e) => return Err(anyhow!("Could not reach {}. Check the internet connection. ({e})", label(provider))),
    };
    if provider == "anthropic" {
        let text = v["content"].as_array().map(|a| a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("")).unwrap_or_default();
        Ok((text, v["usage"]["input_tokens"].as_u64().unwrap_or(0), v["usage"]["output_tokens"].as_u64().unwrap_or(0)))
    } else {
        Ok((v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string(), v["usage"]["prompt_tokens"].as_u64().unwrap_or(0), v["usage"]["completion_tokens"].as_u64().unwrap_or(0)))
    }
}

pub fn edit(c: &Cfg, instruction: &str, f: &Facts) -> Result<Reply> {
    let local = c.provider == "custom";
    if c.key.trim().is_empty() && !local {
        return Err(anyhow!("Add your API key in Settings first"));
    }
    let model = if c.model.trim().is_empty() { default_model(c.provider) } else { c.model.trim() };
    if model.is_empty() {
        return Err(anyhow!("Enter the model name in Settings first"));
    }
    let user = format!("{}\nInstruction: {}", describe(f), instruction.trim());
    let (text, tin, tout) = call(&Cfg { provider: c.provider, key: c.key.trim(), model, base: c.base }, SYSTEM, &user, 3000)?;
    let (explain, ops) = parse(&text, f.duration)?;
    Ok(Reply { explain, ops, tokens_in: tin, tokens_out: tout })
}

pub fn test(c: &Cfg) -> Result<()> {
    let model = if c.model.trim().is_empty() { default_model(c.provider) } else { c.model.trim() };
    if model.is_empty() {
        return Err(anyhow!("Enter the model name first"));
    }
    call(&Cfg { provider: c.provider, key: c.key.trim(), model, base: c.base }, "Reply with this JSON object and nothing else: {\"ok\": true}", "Hello", 64).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_and_the_real_request_both_mention_json() {
        // some providers refuse the JSON switch unless the prompt contains the word "json"
        assert!(SYSTEM.to_lowercase().contains("json"));
        let src = include_str!("ai.rs");
        assert!(src.contains("Reply with this JSON object"));
    }

    #[test]
    fn knows_every_provider_and_builds_custom_addresses() {
        for id in ["anthropic", "openai", "gemini", "openrouter", "deepseek", "mistral", "groq", "xai"] {
            assert!(!default_model(id).is_empty(), "{id}");
            assert!(prov(id).url.starts_with("https://"), "{id}");
        }
        assert_eq!(custom_url("http://localhost:11434/v1/"), "http://localhost:11434/v1/chat/completions");
        assert_eq!(custom_url("http://x/v1/chat/completions"), "http://x/v1/chat/completions");
        assert_eq!(default_model("something else"), default_model("anthropic"));
    }

    fn cfg<'a>(p: &'a str, key: &'a str, model: &'a str, base: &'a str) -> Cfg<'a> {
        Cfg { provider: p, key, model, base }
    }

    #[test]
    fn every_provider_gets_the_request_its_service_expects() {
        let ids = ["anthropic", "openai", "gemini", "openrouter", "deepseek", "mistral", "groq", "xai"];
        for id in ids {
            let r = chat_request(&cfg(id, "K", "m1", ""), "be brief (JSON)", "hi", 100).unwrap();
            assert!(r.url.starts_with("https://"), "{id}");
            let m = models_request(&cfg(id, "K", "m1", "")).unwrap();
            assert!(m.url.ends_with("/models"), "{id}: {}", m.url);
            assert_eq!(r.body["model"], "m1", "{id}");
            if id == "anthropic" {
                assert!(r.url.ends_with("/v1/messages") && m.url == "https://api.anthropic.com/v1/models");
                assert!(r.headers.iter().any(|h| h.0 == "x-api-key" && h.1 == "K") && r.headers.iter().any(|h| h.0 == "anthropic-version"));
                assert_eq!(r.body["system"], "be brief (JSON)");
                assert_eq!(r.body["max_tokens"], 100);
            } else {
                assert!(r.url.ends_with("/chat/completions"), "{id}");
                assert!(r.headers.contains(&("Authorization".to_string(), "Bearer K".to_string())), "{id}");
                assert_eq!(r.body["messages"][0]["role"], "system");
                assert_eq!(r.body["messages"][1]["content"], "hi");
            }
            // the cap is named the way each service wants it
            assert_eq!(r.body.get("max_completion_tokens").is_some(), id == "openai", "{id}");
            assert_eq!(r.body.get("max_tokens").is_some(), id != "openai", "{id}");
        }
        assert_eq!(chat_request(&cfg("gemini", "K", "m", ""), "s", "u", 5).unwrap().url, "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions");
        assert_eq!(models_request(&cfg("gemini", "K", "m", "")).unwrap().url, "https://generativelanguage.googleapis.com/v1beta/openai/models");
        assert_eq!(models_request(&cfg("groq", "K", "m", "")).unwrap().url, "https://api.groq.com/openai/v1/models");
        assert_eq!(models_request(&cfg("openrouter", "", "m", "")).unwrap().url, "https://openrouter.ai/api/v1/models");
        assert_eq!(models_request(&cfg("deepseek", "K", "m", "")).unwrap().url, "https://api.deepseek.com/models");
        // JSON mode only where it is accepted, and the word "json" is always in the prompts
        for id in ["openai", "deepseek", "mistral", "groq"] {
            assert!(chat_request(&cfg(id, "K", "m", ""), "s", "u", 5).unwrap().body.get("response_format").is_some(), "{id}");
        }
        for id in ["gemini", "openrouter", "xai"] {
            assert!(chat_request(&cfg(id, "K", "m", ""), "s", "u", 5).unwrap().body.get("response_format").is_none(), "{id}");
        }
        // a local service needs no key, and accepts the address in either form
        let c = cfg("custom", "", "llama3.1", "http://localhost:11434/v1/");
        let r = chat_request(&c, "s", "u", 5).unwrap();
        assert_eq!(r.url, "http://localhost:11434/v1/chat/completions");
        assert!(r.headers.is_empty());
        assert_eq!(models_request(&cfg("custom", "", "", "http://localhost:11434/v1/chat/completions")).unwrap().url, "http://localhost:11434/v1/models");
        assert!(chat_request(&cfg("custom", "", "m", "localhost"), "s", "u", 5).is_err());
    }

    #[test]
    fn reads_every_providers_model_list_and_picks_a_sensible_default() {
        let openai = json!({"data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini"}, {"id": "text-embedding-3-small"}, {"id": "whisper-1"}, {"id": "dall-e-3"}, {"id": "gpt-5-mini"}]});
        let ids = parse_models(&openai);
        assert!(ids.contains(&"gpt-4o-mini".to_string()) && !ids.iter().any(|i| i.contains("embedding") || i.contains("whisper") || i.contains("dall")));
        assert_eq!(pick_default("openai", &ids).as_deref(), Some("gpt-4o-mini"));
        let gem = json!({"data": [{"id": "models/gemini-2.5-pro"}, {"id": "models/gemini-2.5-flash"}, {"id": "models/gemini-2.5-flash-lite"}, {"id": "models/gemini-3.0-flash"}, {"id": "models/text-embedding-004"}, {"id": "models/imagen-4.0"}]});
        let ids = parse_models(&gem);
        assert!(ids.contains(&"gemini-2.5-flash".to_string()) && !ids.iter().any(|i| i.starts_with("models/")));
        assert_eq!(pick_default("gemini", &ids).as_deref(), Some("gemini-3.0-flash"));
        let ds = parse_models(&json!({"data": [{"id": "deepseek-v4-pro"}, {"id": "deepseek-flash"}]}));
        assert_eq!(pick_default("deepseek", &ds).as_deref(), Some("deepseek-flash"));
        let old_ds = parse_models(&json!({"data": [{"id": "deepseek-chat"}, {"id": "deepseek-reasoner"}]}));
        assert_eq!(pick_default("deepseek", &old_ds).as_deref(), Some("deepseek-chat"));
        let an = parse_models(&json!({"data": [{"id": "claude-opus-5-5"}, {"id": "claude-haiku-5-5"}, {"id": "claude-sonnet-5-5"}], "has_more": false}));
        assert_eq!(pick_default("anthropic", &an).as_deref(), Some("claude-haiku-5-5"));
        let groq = parse_models(&json!({"data": [{"id": "llama-3.3-70b-versatile"}, {"id": "whisper-large-v3"}, {"id": "llama-guard-4-12b"}]}));
        assert_eq!(groq, vec!["llama-3.3-70b-versatile".to_string()]);
        assert_eq!(pick_default("groq", &groq).as_deref(), Some("llama-3.3-70b-versatile"));
        let ollama = parse_models(&json!({"object": "list", "data": [{"id": "llama3.1:latest"}, {"id": "qwen2.5:7b"}]}));
        assert_eq!(pick_default("custom", &ollama).as_deref(), Some("llama3.1:latest"));
        assert!(parse_models(&json!({"error": "x"})).is_empty());
    }

    /// A tiny local server that answers like a provider, so the real network path is exercised end to end.
    fn serve(replies: Vec<(&'static str, String)>) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = vec![0u8; 16384];
                let mut got = Vec::new();
                loop {
                    let n = s.read(&mut buf).unwrap();
                    got.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&got).to_string();
                    if let Some(h) = t.find("\r\n\r\n") {
                        let len = t[..h].to_lowercase().split("content-length:").nth(1).and_then(|x| x.lines().next()).and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
                        if got.len() >= h + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&got).to_string());
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (addr, rx)
    }

    #[test]
    fn talks_to_a_service_end_to_end_and_explains_failures() {
        let reply = json!({"choices": [{"message": {"content": "{\"explain\": \"Cut the start\", \"ops\": [{\"op\": \"cut\", \"from\": 0, \"to\": 5}]}"}}], "usage": {"prompt_tokens": 11, "completion_tokens": 7}}).to_string();
        let models = json!({"data": [{"id": "llama3.1"}, {"id": "nomic-embed-text"}]}).to_string();
        let (base, rx) = serve(vec![("200 OK", models), ("200 OK", reply), ("429 Too Many Requests", json!({"error": {"message": "slow down"}}).to_string())]);
        let c = cfg("custom", "secret", "llama3.1", &base);
        let (ids, pick) = list_models(&c).unwrap();
        assert_eq!(ids, vec!["llama3.1".to_string()]);
        assert_eq!(pick.as_deref(), Some("llama3.1"));
        let seen = rx.recv().unwrap();
        assert!(seen.starts_with("GET /models"), "{seen}");
        assert!(seen.to_lowercase().contains("authorization: bearer secret"));
        let out = edit(&c, "remove the first 5 seconds", &Facts { duration: 60.0, ..Default::default() }).unwrap();
        assert_eq!((out.tokens_in, out.tokens_out), (11, 7));
        assert_eq!(out.ops.len(), 1);
        let seen = rx.recv().unwrap();
        assert!(seen.starts_with("POST /chat/completions"), "{seen}");
        assert!(seen.contains("\"model\":\"llama3.1\"") && seen.contains("max_tokens"));
        let err = edit(&c, "x", &Facts { duration: 60.0, ..Default::default() }).unwrap_err().to_string();
        assert!(err.contains("limit or no credit") && err.contains("slow down"), "{err}");
    }

    #[test]
    fn describes_compactly() {
        let f = Facts { duration: 125.43, markers: vec![10.04], silences: vec![(3.0, 5.55)], idle: vec![], scenes: vec![60.0] };
        let d = describe(&f);
        assert!(d.contains("125.4s") && d.contains("3-5.6") && d.contains("60") && !d.contains("Still-screen"));
    }

    #[test]
    fn estimate_grows_with_arabic_and_stays_small() {
        let f = Facts { duration: 300.0, silences: (0..40).map(|i| (i as f64, i as f64 + 1.0)).collect(), ..Default::default() };
        let en = estimate_tokens("cut the first ten seconds", &f);
        let ar = estimate_tokens("احذف أول عشر ثواني من الفيديو", &f);
        assert!(ar > en && en < 1500, "{en} {ar}");
    }

    #[test]
    fn reads_json_inside_chatter_and_code_fences() {
        let t = "Sure!\n```json\n{\"explain\":\"Cut 0-10 {x}\",\"ops\":[{\"op\":\"cut\",\"from\":0,\"to\":10},{\"op\":\"speed\",\"from\":20,\"to\":30,\"speed\":9},{\"op\":\"text\",\"from\":5,\"text\":\"Hi\"}]}\n```";
        let (e, ops) = parse(t, 100.0).unwrap();
        assert_eq!(e, "Cut 0-10 {x}");
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[1].speed, 4.0);
        assert_eq!(ops[2].to, 8.0);
    }

    #[test]
    fn drops_nonsense_and_clamps() {
        let t = r#"{"explain":"","ops":[{"op":"cut","from":-5,"to":3},{"op":"cut","from":500,"to":600},{"op":"cut","from":90,"to":400},{"op":"fly","from":1,"to":2},{"op":"cut","from":1,"to":1.05}]}"#;
        let (_, ops) = parse(t, 100.0).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!((ops[0].from, ops[0].to), (90.0, 100.0));
        assert!(parse("no json here", 10.0).is_err());
    }

    #[test]
    fn needs_a_key() {
        assert!(edit(&Cfg { provider: "anthropic", key: " ", model: "", base: "" }, "x", &Facts::default()).is_err());
        assert!(edit(&Cfg { provider: "custom", key: "", model: "", base: "" }, "x", &Facts::default()).is_err());
        assert_eq!(default_model("openai"), "gpt-4o-mini");
    }
}
