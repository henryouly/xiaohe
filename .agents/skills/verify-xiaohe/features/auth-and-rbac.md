# Multi-User Auth & RBAC

Xiaohe Core enforces multi-user authentication and area-scoped role-based access control (`admin`, `member`, `guest`) backed by `0600` `config/users.toml`. First-boot initialization creates the initial Admin account via `POST /api/auth/bootstrap`, browser sessions use `HttpOnly; SameSite=Lax` `xiaohe_session` cookies, and headless clients use revocable `xh_*` Bearer tokens verified in constant time with SHA-256.

## Sub-features

- Pre-bootstrap status: `GET /api/auth/me` returns `200 {"initialized":false,"user":null}` before any Admin exists.
- First-boot Admin bootstrap: `POST /api/auth/bootstrap` returns `201` with `Set-Cookie: xiaohe_session=...; HttpOnly; SameSite=Lax; Path=/` and persists `config/users.toml` (`0600`); repeated bootstrap calls return `409 Conflict`.
- Session login & logout: `POST /api/auth/login` verifies Argon2id hashes (`200` + `Set-Cookie`), `POST /api/auth/logout` returns `204` and revokes the server-side session (`401` on reuse).
- Admin user & token administration: `GET/POST /api/users`, `PUT/DELETE /api/users/{user_id}`, `POST /api/users/{user_id}/tokens`, `DELETE /api/users/{user_id}/tokens/{token_id}` reject unauthenticated callers (`401`) and non-Admin callers (`403`), never expose password or token hashes, and protect the last remaining Admin from deletion or demotion (`409`).

## How to get to it (user POV)

Send HTTP requests to `http://127.0.0.1:$PORT/api/auth/*` and `http://127.0.0.1:$PORT/api/users/*` using a cookie jar (`-c`/`-b`) for session auth or `Authorization: Bearer xh_...` for API tokens.

## Driving it with `xiaohe-verify.sh` + `curl`

Preconditions: Fresh `xiaohe supervisor` + `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"` (no `users.toml` yet) and `xiaohe-verify.sh doctor "$RUN_ID"` passed.

```bash
BASE="http://127.0.0.1:${PORT}"
JAR="${SCRATCH_DIR}/admin.cookies"

# 1. Pre-bootstrap /api/auth/me -> 200 initialized=false; unauthenticated /api/users -> 401
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/auth/me" | tee "$EVIDENCE_DIR/auth-me-pre.txt"
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/api/users" | tee "$EVIDENCE_DIR/auth-users-unauth.txt"

# 2. First bootstrap -> 201 + HttpOnly Set-Cookie; second bootstrap -> 409
curl -sS -D "$EVIDENCE_DIR/auth-boot.hdr" -c "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' \
  -X POST "$BASE/api/auth/bootstrap" -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"password123","display_name":"Admin"}' \
  | tee "$EVIDENCE_DIR/auth-boot-1.txt"

ADMIN_ID=$(head -n1 "$EVIDENCE_DIR/auth-boot-1.txt" | jq -r '.user.id')

curl -sS -w '\nHTTP_STATUS:%{http_code}\n' \
  -X POST "$BASE/api/auth/bootstrap" -H 'Content-Type: application/json' \
  -d '{"username":"admin2","password":"password123","display_name":"Admin 2"}' \
  | tee "$EVIDENCE_DIR/auth-boot-2.txt"

# 3. Admin creates Member & mints Bearer token; Member gets 403 on /api/users, 200 on /api/auth/me
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' \
  -X POST "$BASE/api/users" -H 'Content-Type: application/json' \
  -d '{"username":"alice","display_name":"Alice","password":"password123","role":"member","allowed_areas":["living_room"]}' \
  | tee "$EVIDENCE_DIR/auth-create-member.txt"

MEMBER_ID=$(head -n1 "$EVIDENCE_DIR/auth-create-member.txt" | jq -r '.user.id')

curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' \
  -X POST "$BASE/api/users/${MEMBER_ID}/tokens" -H 'Content-Type: application/json' \
  -d '{"name":"cli"}' | tee "$EVIDENCE_DIR/auth-mint-token.txt"

MEMBER_TOK=$(head -n1 "$EVIDENCE_DIR/auth-mint-token.txt" | jq -r '.token')

curl -sS -w '\nHTTP_STATUS:%{http_code}\n' \
  -H "Authorization: Bearer ${MEMBER_TOK}" "$BASE/api/users" \
  | tee "$EVIDENCE_DIR/auth-member-forbidden.txt"

curl -sS -w '\nHTTP_STATUS:%{http_code}\n' \
  -H "Authorization: Bearer ${MEMBER_TOK}" "$BASE/api/auth/me" \
  | tee "$EVIDENCE_DIR/auth-member-me.txt"

# 4. Deleting last remaining Admin -> 409; logout -> 204 and invalidates cookie (-> 401)
curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' \
  -X DELETE "$BASE/api/users/${ADMIN_ID}" | tee "$EVIDENCE_DIR/auth-delete-last-admin.txt"

curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' \
  -X POST "$BASE/api/auth/logout" | tee "$EVIDENCE_DIR/auth-logout.txt"

curl -sS -b "$JAR" -w '\nHTTP_STATUS:%{http_code}\n' \
  "$BASE/api/users" | tee "$EVIDENCE_DIR/auth-after-logout.txt"

# 5. Confirm users.toml persisted with 0600 permissions
stat -c '%a %n' "$SCRATCH_DIR/config/users.toml" | tee "$EVIDENCE_DIR/auth-users-toml-mode.txt"
```

## Gotchas

- `POST /api/auth/bootstrap` is one-time only per `CONFIG_DIR`; once an Admin exists in `users.toml`, log in via `POST /api/auth/login` instead.
- Passwords must be at least 8 characters and usernames 2–64 characters matching `[a-zA-Z0-9_.-]`.
- `POST /api/users/{user_id}/tokens` returns the plaintext `xh_*` token once; only its `sha256:` digest is stored in `users.toml`.
- Changing a user's password or role revokes all of that user's active sessions immediately (including open SSE streams).
