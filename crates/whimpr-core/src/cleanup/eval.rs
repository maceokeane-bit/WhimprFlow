//! Fixture-driven cleanup regression harness.
//!
//! Cases live in `evals/cleanup_cases.json`. They exercise deterministic pieces
//! (gates, layout pre/post, prompt assembly) without calling an LLM — so CI can
//! block prompt/gate regressions on every push.
//!
//! Run via `cargo test -p whimpr-core cleanup_eval` or
//! `cargo run -p whimpr-core --example cleanup_eval`.

use serde::Deserialize;

use super::gates::{evaluate, GateReason, GateVerdict};
use super::levels::CleanupLevel;
use super::{assemble_user_message, post_process, pre_normalize_layout, CleanupContext, VocabEntry};

const FIXTURES: &str = include_str!("../../evals/cleanup_cases.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalKind {
    Gate,
    PostProcess,
    LayoutPipeline,
    AssembleIncludes,
    AssembleExcludes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expect {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonTag {
    BannedPattern,
    LostEntity,
    OverDeletion,
    Hallucination,
    EditRatio,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub category: String,
    pub kind: EvalKind,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub cleaned: Option<String>,
    #[serde(default)]
    pub expect: Option<Expect>,
    #[serde(default)]
    pub reason: Option<ReasonTag>,
    #[serde(default)]
    pub expect_cleaned: Option<String>,
    #[serde(default)]
    pub window_context: Option<String>,
    #[serde(default)]
    pub app_bundle_id: Option<String>,
    #[serde(default)]
    pub vocab_correct: Option<String>,
    #[serde(default)]
    pub vocab_mishears: Option<Vec<String>>,
    #[serde(default)]
    pub needle: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CaseFailure {
    pub id: String,
    pub category: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct EvalReport {
    pub total: usize,
    pub passed: usize,
    pub failed: Vec<CaseFailure>,
}

impl EvalReport {
    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }
}

fn parse_level(raw: Option<&str>) -> CleanupLevel {
    match raw.unwrap_or("light") {
        "none" => CleanupLevel::None,
        "medium" => CleanupLevel::Medium,
        "high" => CleanupLevel::High,
        _ => CleanupLevel::Light,
    }
}

fn reason_matches(tag: &ReasonTag, reason: &GateReason) -> bool {
    match (tag, reason) {
        (ReasonTag::BannedPattern, GateReason::BannedPattern(_)) => true,
        (ReasonTag::LostEntity, GateReason::LostEntity(_)) => true,
        (ReasonTag::OverDeletion, GateReason::OverDeletion { .. }) => true,
        (ReasonTag::Hallucination, GateReason::Hallucination) => true,
        (ReasonTag::EditRatio, GateReason::EditRatioTooHigh { .. }) => true,
        _ => false,
    }
}

fn build_ctx(case: &EvalCase) -> CleanupContext {
    let mut ctx = CleanupContext {
        level: parse_level(case.level.as_deref()),
        window_context: case.window_context.clone(),
        app_bundle_id: case.app_bundle_id.clone(),
        ..Default::default()
    };
    if let Some(correct) = &case.vocab_correct {
        ctx.vocab.push(VocabEntry {
            correct: correct.clone(),
            mishears: case.vocab_mishears.clone().unwrap_or_default(),
        });
    }
    ctx
}

fn run_case(case: &EvalCase) -> Result<(), String> {
    match case.kind {
        EvalKind::Gate => {
            let raw = case.raw.as_deref().ok_or("missing raw")?;
            let cleaned = case.cleaned.as_deref().ok_or("missing cleaned")?;
            let expect = case.expect.ok_or("missing expect")?;
            let level = parse_level(case.level.as_deref());
            let verdict = evaluate(raw, cleaned, level);
            match expect {
                Expect::Pass => {
                    if verdict.passed() {
                        Ok(())
                    } else {
                        Err(format!("expected pass, got {verdict:?}"))
                    }
                }
                Expect::Fail => match verdict {
                    GateVerdict::Fail(reason) => {
                        if let Some(tag) = &case.reason {
                            if reason_matches(tag, &reason) {
                                Ok(())
                            } else {
                                Err(format!("expected reason {tag:?}, got {reason:?}"))
                            }
                        } else {
                            Ok(())
                        }
                    }
                    GateVerdict::Pass => Err("expected fail, got pass".into()),
                },
            }
        }
        EvalKind::PostProcess => {
            let raw = case.raw.as_deref().ok_or("missing raw")?;
            let expected = case.expect_cleaned.as_deref().ok_or("missing expect_cleaned")?;
            let got = post_process(raw);
            if got == expected {
                Ok(())
            } else {
                Err(format!("post_process mismatch\n  got: {got:?}\n  exp: {expected:?}"))
            }
        }
        EvalKind::LayoutPipeline => {
            let raw = case.raw.as_deref().ok_or("missing raw")?;
            let expected = case.expect_cleaned.as_deref().ok_or("missing expect_cleaned")?;
            let got = post_process(&pre_normalize_layout(raw));
            if got == expected {
                Ok(())
            } else {
                Err(format!(
                    "layout pipeline mismatch\n  got: {got:?}\n  exp: {expected:?}"
                ))
            }
        }
        EvalKind::AssembleIncludes => {
            let raw = case.raw.as_deref().ok_or("missing raw")?;
            let needle = case.needle.as_deref().ok_or("missing needle")?;
            let msg = assemble_user_message(raw, &build_ctx(case));
            if msg.contains(needle) {
                Ok(())
            } else {
                Err(format!("expected assembled message to contain {needle:?}"))
            }
        }
        EvalKind::AssembleExcludes => {
            let raw = case.raw.as_deref().ok_or("missing raw")?;
            let needle = case.needle.as_deref().ok_or("missing needle")?;
            let msg = assemble_user_message(raw, &build_ctx(case));
            if msg.contains(needle) {
                Err(format!("expected assembled message to omit {needle:?}"))
            } else {
                Ok(())
            }
        }
    }
}

/// Load the embedded fixture file.
pub fn load_cases() -> Result<Vec<EvalCase>, String> {
    serde_json::from_str(FIXTURES).map_err(|e| format!("invalid cleanup_cases.json: {e}"))
}

/// Also assert every few-shot demonstration pair passes Light gates.
fn few_shot_cases() -> Vec<EvalCase> {
    super::prompts::FEW_SHOT
        .iter()
        .enumerate()
        .map(|(i, (raw, cleaned))| EvalCase {
            id: format!("few_shot_{i:02}"),
            category: "few_shot".into(),
            kind: EvalKind::Gate,
            level: Some("light".into()),
            raw: Some((*raw).to_string()),
            cleaned: Some((*cleaned).to_string()),
            expect: Some(Expect::Pass),
            reason: None,
            expect_cleaned: None,
            window_context: None,
            app_bundle_id: None,
            vocab_correct: None,
            vocab_mishears: None,
            needle: None,
        })
        .collect()
}

/// Run the full deterministic cleanup regression suite.
pub fn run_eval() -> Result<EvalReport, String> {
    let mut cases = load_cases()?;
    cases.extend(few_shot_cases());
    let total = cases.len();
    let mut failed = Vec::new();
    for case in &cases {
        if let Err(detail) = run_case(case) {
            failed.push(CaseFailure {
                id: case.id.clone(),
                category: case.category.clone(),
                detail,
            });
        }
    }
    Ok(EvalReport {
        total,
        passed: total - failed.len(),
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_eval_fixtures_pass() {
        let report = run_eval().expect("fixtures parse");
        if !report.ok() {
            let mut msg = format!(
                "cleanup eval: {}/{} passed; failures:\n",
                report.passed, report.total
            );
            for f in &report.failed {
                msg.push_str(&format!("  - [{}] {}: {}\n", f.category, f.id, f.detail));
            }
            panic!("{msg}");
        }
        assert!(report.total >= 40, "expected a useful fixture count");
    }
}
