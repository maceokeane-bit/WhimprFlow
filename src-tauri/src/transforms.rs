//! Run transform rewrites on selected text via Ollama.

use whimpr_core::transforms::TRANSFORM_SYSTEM;

pub fn apply_via_ollama(
    selected: &str,
    instruction: &str,
    ollama_base: &str,
    ollama_model: &str,
) -> Result<String, String> {
    let user = if selected.trim().is_empty() {
        format!(
            "Instruction: {instruction}\n\nNo text is selected. Generate the requested content \
             for insertion at the cursor. Return only that content."
        )
    } else {
        format!("Instruction: {instruction}\n\n<SELECTED_TEXT>\n{selected}\n</SELECTED_TEXT>")
    };
    let messages = whimpr_cleanup::ollama::simple_messages(TRANSFORM_SYSTEM, &user);

    whimpr_cleanup::ollama::chat(ollama_base, ollama_model, &messages).map_err(|e| e.to_string())
}
