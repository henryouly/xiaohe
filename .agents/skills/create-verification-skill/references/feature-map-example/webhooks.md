# HTTP Webhooks

HTTP Webhooks let external services (iOS Shortcuts, NAS alerts, scripts) trigger Xiaohe over `POST /api/webhooks/{webhook_id}` with per-webhook SHA-256 secret verification and optional JSON payloads up to 64 KB.

## Sub-features

- `webhook-bearer` accepts a valid `Authorization: Bearer <secret>` request with a JSON body and returns `202`.
- `webhook-header` accepts a valid `X-Webhook-Secret: <secret>` request with an empty body and returns `202`.
- `webhook-reject-auth` rejects missing or wrong secrets with `401`.
- `webhook-reject-unknown` rejects unknown or disabled webhook IDs with `404`.
- `webhook-reject-body` rejects malformed JSON with `400` and payloads over 64 KB with `413`.

## How to get to it (user POV)

- Configure `[[webhooks]]` in `$CONFIG_DIR/system.toml` with `id`, `secret_hash` (`sha256:<hex>`), and `enabled = true`.
- Send `POST http://127.0.0.1:$PORT/api/webhooks/{webhook_id}` with either `Authorization: Bearer <secret>` or `X-Webhook-Secret: <secret>`.

## Driving it with curl

Preconditions:

- `xiaohe core` is healthy at `http://127.0.0.1:$PORT` (`Doctor` passed).
- `$CONFIG_DIR/system.toml` defines webhook `id = "verify_hook"` (`enabled = true`) with `secret_hash = "sha256:$(printf '%s' 'verify-secret' | sha256sum | awk '{print $1}')"`.

- **Valid Bearer + JSON.** Send `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST http://127.0.0.1:$PORT/api/webhooks/verify_hook -H 'Authorization: Bearer verify-secret' -H 'Content-Type: application/json' -d '{"event":"arrived"}' | tee "$EVIDENCE_DIR/webhook-bearer.txt"`. Body is `{"accepted":true,"webhook_id":"verify_hook"}` and `HTTP_STATUS:202`.
- **Valid header + empty body.** Send `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST http://127.0.0.1:$PORT/api/webhooks/verify_hook -H 'X-Webhook-Secret: verify-secret' | tee "$EVIDENCE_DIR/webhook-header.txt"`. Body is `{"accepted":true,"webhook_id":"verify_hook"}` and `HTTP_STATUS:202`.
- **Wrong secret.** Send `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST http://127.0.0.1:$PORT/api/webhooks/verify_hook -H 'X-Webhook-Secret: wrong' | tee "$EVIDENCE_DIR/webhook-401.txt"`. Body is `{"error":"unauthorized"}` and `HTTP_STATUS:401`.
- **Unknown webhook.** Send `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST http://127.0.0.1:$PORT/api/webhooks/no_such_hook -H 'X-Webhook-Secret: verify-secret' | tee "$EVIDENCE_DIR/webhook-404.txt"`. Body is `{"error":"not_found"}` and `HTTP_STATUS:404`.
- **Malformed JSON.** Send `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST http://127.0.0.1:$PORT/api/webhooks/verify_hook -H 'X-Webhook-Secret: verify-secret' -d '{bad' | tee "$EVIDENCE_DIR/webhook-400.txt"`. Body is `{"error":"invalid_json"}` and `HTTP_STATUS:400`.

## Gotchas

- `system.toml` is loaded at startup; editing `[[webhooks]]` requires restarting `xiaohe core` before driving.
- `printf '%s'` (not `echo`) must be used when computing `sha256sum` so a trailing newline is not hashed into `secret_hash`.
- Bodies larger than 65,536 bytes are rejected by Axum with `413` before the webhook handler runs.
