//! Local-LLM cleanup worker.
//!
//! Loads a GGUF instruction model once, then serves one request per line of stdin:
//! `{"system": "...", "user": "..."}` → `{"text": "..."}` on stdout. The WhimprFlow
//! app spawns this and keeps it warm so cleanup is fast and fully offline.
//!
//! Usage: `whimpr-llm-worker <model.gguf>` (or WHIMPR_LLM_MODEL env var).

use std::io::{BufRead, Write};
use std::num::NonZeroU32;

use anyhow::Context as _;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Msg {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct Request {
    /// Full multi-turn message list (system + few-shot + user). Preferred.
    #[serde(default)]
    messages: Vec<Msg>,
    /// Back-compat single-turn form, used only when `messages` is empty.
    #[serde(default)]
    system: String,
    #[serde(default)]
    user: String,
    #[serde(default = "default_max")]
    max_tokens: i32,
}
fn default_max() -> i32 {
    400
}

#[derive(Serialize)]
struct Response {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model_path = args
        .get(1)
        .cloned()
        .or_else(|| std::env::var("WHIMPR_LLM_MODEL").ok())
        .context("model path required (argv[1] or WHIMPR_LLM_MODEL)")?;

    let backend = LlamaBackend::init()?;
    // Offload everything to the Apple GPU (Metal) — capped by what fits.
    let model_params = LlamaModelParams::default().with_n_gpu_layers(999);
    let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
        .with_context(|| format!("failed to load model {model_path}"))?;
    eprintln!("[llm-worker] model loaded, ready");
    if args.iter().any(|arg| arg == "--eval") {
        return run_eval(&backend, &model);
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => match generate(&backend, &model, &req) {
                Ok(text) => Response { text, error: None },
                Err(e) => Response {
                    text: String::new(),
                    error: Some(e.to_string()),
                },
            },
            Err(e) => Response {
                text: String::new(),
                error: Some(format!("bad request: {e}")),
            },
        };
        serde_json::to_writer(&mut stdout, &resp)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

fn run_eval(backend: &LlamaBackend, model: &LlamaModel) -> anyhow::Result<()> {
    let cases = [
        "um so i think we should uh meet at 3",
        "send the invoice to account 12345 for 500 dollars",
        "what time is the standup",
        "first item new line second item new line third item",
        "i can meet monday no actually make that tuesday afternoon",
    ];
    let mut passed = 0_u32;
    let mut results = Vec::new();
    for raw in cases {
        let context = whimpr_core::CleanupContext::default();
        let messages = whimpr_core::cleanup::build_messages(raw, &context)
            .into_iter()
            .map(|message| Msg {
                role: message.role.to_string(),
                content: message.content,
            })
            .collect();
        let request = Request {
            messages,
            system: String::new(),
            user: String::new(),
            max_tokens: 400,
        };
        let generated = generate(backend, model, &request)?;
        let cleaned = whimpr_core::cleanup::post_process(&generated);
        let verdict = whimpr_core::cleanup::evaluate_gates(raw, &cleaned, context.level);
        let ok = verdict.passed() && !cleaned.trim().is_empty();
        if ok {
            passed += 1;
        }
        results.push(serde_json::json!({
            "raw": raw,
            "cleaned": cleaned,
            "passed": ok,
            "verdict": format!("{verdict:?}"),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "passed": passed,
            "total": results.len(),
            "results": results,
        }))?
    );
    if passed as usize != results.len() {
        anyhow::bail!("local cleanup evaluation failed");
    }
    Ok(())
}

fn generate(backend: &LlamaBackend, model: &LlamaModel, req: &Request) -> anyhow::Result<String> {
    // Qwen2.5 ChatML template. Prefer the full multi-turn message list (few-shot
    // demonstrations drive the newline/list/self-correction behavior); fall back
    // to the legacy single system+user pair.
    let mut prompt = String::new();
    if req.messages.is_empty() {
        prompt.push_str(&format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n",
            req.system, req.user
        ));
    } else {
        for m in &req.messages {
            prompt.push_str(&format!(
                "<|im_start|>{}\n{}<|im_end|>\n",
                m.role, m.content
            ));
        }
    }
    prompt.push_str("<|im_start|>assistant\n");

    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(4096));
    let mut ctx = model.new_context(backend, ctx_params)?;

    let tokens = model.str_to_token(&prompt, AddBos::Always)?;
    let n_prompt = tokens.len() as i32;

    let mut batch = LlamaBatch::new(4096, 1);
    let last = tokens.len() - 1;
    for (i, tok) in tokens.iter().enumerate() {
        batch.add(*tok, i as i32, &[0], i == last)?;
    }
    ctx.decode(&mut batch)?;

    let mut sampler = LlamaSampler::greedy();
    let mut n_cur = batch.n_tokens();
    let mut out = String::new();
    let limit = n_prompt + req.max_tokens;

    while n_cur <= limit {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        out.push_str(&model.token_to_str(token, Special::Tokenize)?);
        batch.clear();
        batch.add(token, n_cur, &[0], true)?;
        n_cur += 1;
        ctx.decode(&mut batch)?;
    }
    Ok(out.trim().to_string())
}
