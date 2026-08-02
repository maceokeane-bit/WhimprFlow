//! Voice insights: analyze stored dictation history via Ollama.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::local_llm;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InsightReport {
    pub generated_at: u64,
    pub sessions_analyzed: u32,
    pub reading_grade: String,
    pub complexity: String,
    pub domain_depth: String,
    pub summary: String,
    pub topics: Vec<String>,
    pub vocabulary_note: String,
    #[serde(default)]
    pub error: Option<String>,
}

fn insights_path() -> PathBuf {
    local_llm::app_support_dir().join("insights.json")
}

pub fn load_cached() -> Option<InsightReport> {
    let path = insights_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save_cached(report: &InsightReport) {
    let path = insights_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(report).unwrap_or_default());
}

/// Run (or return cached) language analysis over recent dictation text.
pub fn analyze(
    sessions: &[(String, Option<String>)], // (text, app)
    ollama_base: &str,
    ollama_model: &str,
    force_refresh: bool,
) -> InsightReport {
    if !force_refresh {
        if let Some(cached) = load_cached() {
            // Reuse if less than 1 hour old and we have sessions.
            let age = unix_now().saturating_sub(cached.generated_at);
            if age < 3600 && cached.sessions_analyzed > 0 {
                return cached;
            }
        }
    }

    let mut report = InsightReport {
        generated_at: unix_now(),
        ..Default::default()
    };

    if sessions.is_empty() {
        report.error = Some("No dictation history yet — speak a few times first.".into());
        return report;
    }

    report.sessions_analyzed = sessions.len() as u32;

    let sample: String = sessions
        .iter()
        .take(40)
        .map(|(t, app)| {
            let app = app.as_deref().unwrap_or("unknown");
            format!("[{app}] {t}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let system = "You analyze someone's dictated writing samples. Respond with ONLY valid JSON, no markdown fences.";
    let user = format!(
        r#"Analyze these dictated text samples from one person. Estimate:
1. reading_grade — US school grade level of their spoken vocabulary (e.g. "8th grade", "college")
2. complexity — low / medium / high (conceptual density)
3. domain_depth — shallow / moderate / deep (how expert they sound on their topics)
4. summary — 2-3 sentences on how they communicate
5. topics — array of 3-6 main themes
6. vocabulary_note — one sentence on distinctive word choices

Samples:
{sample}

JSON shape:
{{"reading_grade":"","complexity":"","domain_depth":"","summary":"","topics":[],"vocabulary_note":""}}"#
    );

    let body = serde_json::json!({
        "model": ollama_model,
        "think": false,
        "stream": false,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "options": {"temperature": 0.3, "num_predict": 600},
    });

    let chat_url = whimpr_cleanup::ollama::native_chat_url(ollama_base);

    let out = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "120", "-X", "POST", &chat_url])
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-d")
        .arg(body.to_string())
        .output();

    match out {
        Ok(o) if o.status.success() => {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&o.stdout) {
                let content = v["message"]["content"]
                    .as_str()
                    .or_else(|| v["choices"][0]["message"]["content"].as_str())
                    .unwrap_or("")
                    .trim();
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
                    report.reading_grade = parsed["reading_grade"].as_str().unwrap_or("—").to_string();
                    report.complexity = parsed["complexity"].as_str().unwrap_or("—").to_string();
                    report.domain_depth = parsed["domain_depth"].as_str().unwrap_or("—").to_string();
                    report.summary = parsed["summary"].as_str().unwrap_or("").to_string();
                    report.vocabulary_note =
                        parsed["vocabulary_note"].as_str().unwrap_or("").to_string();
                    report.topics = parsed["topics"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    save_cached(&report);
                    return report;
                }
            }
            report.error = Some("Could not parse Ollama response.".into());
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            report.error = Some(format!(
                "Ollama analysis failed — is it running? {err}"
            ));
        }
        Err(e) => {
            report.error = Some(format!("Could not reach Ollama: {e}"));
        }
    }

    report
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
