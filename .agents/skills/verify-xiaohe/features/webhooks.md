# HTTP Webhooks

HTTP Webhooks let external callers trigger Xiaohe via `POST /api/webhooks/{webhook_id}` with constant-time SHA-256 secret verification and optional JSON payloads up to 64 KB.

## Sub-features

- `webhook-bearer` accepts `Authorization: Bearer <secret>` + JSON body and returns `202`.
- `webhook-header` accepts `X-Webhook-Secret: <secret>` + empty body and returns `202`.
- `webhook-reject-auth` rejects missing or invalid secrets with `401`.
- `webhook-reject-unknown` rejects unknown or disabled webhook IDs with `404`.
- `webhook-reject-body` rejects malformed JSON with `400` and payloads > 64 KB with `413`.

## How to get to it (user POV)

- Define `[[webhooks]]` in `$CONFIG_DIR/system.toml`.
- Send `POST http://127.0.0.1:$PORT/api/webhooks/{webhook_id}` with `Authorization: Bearer <secret>` or `X-Webhook-Secret: <secret>`.

## Driving it with curl

Preconditions:

- `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"` and `doctor` passed (seeds `verify_hook` with secret `verify-secret` and `disabled_hook`).

- **Valid Bearer + JSON.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST "http://127.0.0.1:${PORT}/api/webhooks/verify_hook" -H 'Authorization: Bearer verify-secret' -H 'Content-Type: application/json' -d '{"person":"alice"}' | tee "${EVIDENCE_DIR}/webhook-bearer.txt"`. Body is `{"accepted":true,"webhook_id":"verify_hook"}` and `HTTP_STATUS:202`.
- **Valid X-Webhook-Secret + empty body.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST "http://127.0.0.1:${PORT}/api/webhooks/verify_hook" -H 'X-Webhook-Secret: verify-secret' | tee "${EVIDENCE_DIR}/webhook-header.txt"`. Body is `{"accepted":true,"webhook_id":"verify_hook"}` and `HTTP_STATUS:202`.
- **Invalid secret.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST "http://127.0.0.1:${PORT}/api/webhooks/verify_hook" -H 'X-Webhook-Secret: wrong-secret' | tee "${EVIDENCE_DIR}/webhook-401.txt"`. Body is `{"error":"unauthorized"}` and `HTTP_STATUS:401`.
- **Disabled & unknown webhooks.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST "http://127.0.0.1:${PORT}/api/webhooks/disabled_hook" -H 'X-Webhook-Secret: verify-secret' | tee "${EVIDENCE_DIR}/webhook-404.txt"`. Body is `{"error":"not_found"}` and `HTTP_STATUS:404`.
- **Malformed JSON.** Run `curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST "http://127.0.0.1:${PORT}/api/webhooks/verify_hook" -H 'X-Webhook-Secret: verify-secret' -d '{invalid' | tee "${EVIDENCE_DIR}/webhook-400.txt"`. Body is `{"error":"invalid_json"}` and `HTTP_STATUS:400`.
- **Oversized body (> 64 KB).** Run `python3 -c 'print(" " * 65537, end="")' | curl -sS -w '\nHTTP_STATUS:%{http_code}\n' -X POST "http://127.0.0.1:${PORT}/api/webhooks/verify_hook" -H 'X-Webhook-Secret: verify-secret' --data-binary @- | tee "${EVIDENCE_DIR}/webhook-413.txt"`. Output contains `HTTP_STATUS:413`.

## Gotchas

- `[[webhooks]]` is loaded at startup; changes to `system.toml` require restarting `xiaohe core`.
- Use `printf '%s'` rather than `echo` when hashing secrets so no trailing newline is included.
- Axum rejects > 64 KB bodies with `413` before `handle_webhook` runs (plain text response body, not JSON).
