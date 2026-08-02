#!/bin/bash
# Quick smoke test: Ollama reachable + qwen3:8b cleanup path works.
set -euo pipefail

MODEL="${WHIMPR_OLLAMA_MODEL:-qwen3:8b}"
CHAT_URL="http://localhost:11434/api/chat"

echo "== Ollama tags =="
TAGS=$(curl -sf --max-time 3 http://localhost:11434/api/tags) || {
  echo "FAIL: Ollama not running — open the Ollama app first."
  exit 1
}
echo "$TAGS" | python3 -c "import json,sys; m=[x['name'] for x in json.load(sys.stdin).get('models',[])]; print('  installed:', ', '.join(m))"
echo "$TAGS" | python3 -c "import json,sys; m=[x['name'] for x in json.load(sys.stdin).get('models',[])]; sys.exit(0 if any('$MODEL'==x or x.startswith('$MODEL:') for x in m) else 1)" || {
  echo "FAIL: $MODEL not in ollama list"
  exit 1
}
echo "OK: $MODEL is installed"

echo ""
echo "== Cleanup request ($MODEL, think:false) =="
RAW="um so i think we should meet at two actually three period does that work question mark"
RESP=$(curl -sf --max-time 120 -X POST "$CHAT_URL" \
  -H "Content-Type: application/json" \
  -d "$(python3 - <<PY
import json
print(json.dumps({
  "model": "$MODEL",
  "think": False,
  "stream": False,
  "messages": [
    {"role": "system", "content": "You clean spoken dictation. Return ONLY the cleaned text. Resolve self-corrections (keep three, drop two)."},
    {"role": "user", "content": """$RAW"""}
  ],
  "options": {"temperature": 0.2, "num_predict": 120}
}))
PY
)")

CLEANED=$(echo "$RESP" | python3 -c "import json,sys; print(json.load(sys.stdin)['message']['content'].strip())")
echo "  raw:     $RAW"
echo "  cleaned: $CLEANED"

if [[ -z "$CLEANED" ]]; then
  echo "FAIL: empty cleanup response"
  exit 1
fi
if [[ "$CLEANED" == *"um"* ]] || [[ "$CLEANED" == *"actually two"* ]]; then
  echo "WARN: cleanup may be weak — but model responded"
else
  echo "OK: cleanup looks reasonable"
fi

echo ""
echo "== Settings =="
SETTINGS="$HOME/Library/Application Support/WhimprFlow/settings.json"
if [[ -f "$SETTINGS" ]]; then
  python3 - <<PY
import json, pathlib
d = json.loads(pathlib.Path("$SETTINGS").read_text())
print("  ollama_model:", d.get("ollama_model"))
assert d.get("ollama_model") == "qwen3:8b", "expected qwen3:8b in settings"
print("OK: settings point at qwen3:8b")
PY
else
  echo "  (no settings file — fresh install defaults to qwen3:8b)"
fi

echo ""
echo "All smoke checks passed."
