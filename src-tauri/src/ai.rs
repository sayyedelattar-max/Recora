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
    Prov { id: "deepseek", url: "https://api.deepseek.com/chat/completions", model: "deepseek-chat", json_mode: true },
    Prov { id: "mistral", url: "https://api.mistral.ai/v1/chat/completions", model: "mistral-small-latest", json_mode: true },
    Prov { id: "groq", url: "https://api.groq.com/openai/v1/chat/completions", model: "llama-3.3-70b-versatile", json_mode: true },
    Prov { id: "xai", url: "https://api.x.ai/v1/chat/completions", model: "grok-3-mini", json_mode: false },
    // any service that speaks the same chat format, including a model running on this computer (Ollama, LM Studio)
    Prov { id: "custom", url: "", model: "", json_mode: false },
];

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

fn call(c: &Cfg, system: &str, user: &str, max_tokens: u32) -> Result<(String, u64, u64)> {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(90)).build();
    let p = prov(c.provider);
    let (provider, key, model) = (c.provider, c.key, c.model);
    let resp = if provider == "anthropic" {
        agent
            .post(p.url)
            .set("x-api-key", key)
            .set("anthropic-version", "2023-06-01")
            .send_json(json!({"model": model, "max_tokens": max_tokens, "system": system, "messages": [{"role": "user", "content": user}]}))
    } else {
        let url = if provider == "custom" { custom_url(c.base) } else { p.url.to_string() };
        if provider == "custom" && !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(anyhow!("Enter the address of the service first, for example http://localhost:11434/v1"));
        }
        let mut body = json!({"model": model, "max_tokens": max_tokens, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]});
        if p.json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        let mut r = agent.post(&url);
        if !key.is_empty() {
            r = r.set("Authorization", &format!("Bearer {key}"));
        }
        r.send_json(body)
    };
    let v: Value = match resp {
        Ok(r) => r.into_json()?,
        Err(ureq::Error::Status(code, r)) => {
            let body: Value = r.into_json().unwrap_or(Value::Null);
            let msg = body["error"]["message"].as_str().or_else(|| body["error"].as_str()).or_else(|| body[0]["error"]["message"].as_str()).unwrap_or("").to_string();
            return Err(anyhow!(match code {
                401 | 403 => "The key was not accepted. Check that you copied all of it.".to_string(),
                402 | 429 => format!("The provider refused the request (limit or no credit). {msg}"),
                404 => format!("That model name or address is not available for this key. {msg}"),
                _ => format!("The provider returned an error ({code}). {msg}"),
            }));
        }
        Err(e) => return Err(anyhow!("Could not reach the provider. Check the internet connection. ({e})")),
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
    let (text, tin, tout) = call(&Cfg { provider: c.provider, key: c.key.trim(), model, base: c.base }, SYSTEM, &user, 900)?;
    let (explain, ops) = parse(&text, f.duration)?;
    Ok(Reply { explain, ops, tokens_in: tin, tokens_out: tout })
}

pub fn test(c: &Cfg) -> Result<()> {
    let model = if c.model.trim().is_empty() { default_model(c.provider) } else { c.model.trim() };
    if model.is_empty() {
        return Err(anyhow!("Enter the model name first"));
    }
    call(&Cfg { provider: c.provider, key: c.key.trim(), model, base: c.base }, "Reply with the single word OK.", "Hello", 8).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

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
