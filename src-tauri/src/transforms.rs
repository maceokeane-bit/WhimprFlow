//! Run transform rewrites on selected text via Ollama.

use whimpr_core::transforms::TRANSFORM_SYSTEM;

pub fn apply_via_ollama(
    selected: &str,
    instruction: &str,
    ollama_base: &str,
    ollama_model: &str,
) -> Result<String, String> {
    if selected.trim().is_empty() {
        return Err("Selected text is empty".into());
    }

    let user = format!(
        "Instruction: {instruction}\n\n<SELECTED_TEXT>\n{selected}\n</SELECTED_TEXT>"
    );
    let messages = whimpr_cleanup::ollama::simple_messages(TRANSFORM_SYSTEM, &user);

    whimpr_cleanup::ollama::chat(ollama_base, ollama_model, &messages).map_err(|e| e.to_string())
}
