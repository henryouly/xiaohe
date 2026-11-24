# SolidJS Web Dashboard & SPA Static Serving

Xiaohe Core serves the compiled SolidJS + Tailwind CSS v4 + `uPlot` single-page application (`web/dist`, < 200 KB gzipped) from `WEB_DIST_DIR` (or local `./web/dist` when present), falling back to `index.html` (`200 OK`) for client-side routes (`/`, `/history`, `/skills`, `/automations`) while strictly isolating `/api`, `/api/`, and `/api/*` unknown routes to JSON `404 {"error":"not_found"}` and missing `/assets/*` files to `404`.

## Sub-features

- **SPA static serving & fallback**: `GET /`, `/history`, `/skills`, `/automations` return `200 OK` `text/html` (`index.html`).
- **Hashed asset serving & 404 isolation**: `GET /assets/<built-chunk>.js` and `.css` return `200 OK` with matching MIME types; missing `/assets/nonexistent.js` returns `404 Not Found` (not `index.html`).
- **API 404 isolation**: `GET /api`, `GET /api/`, `POST /api/`, and `GET /api/does_not_exist` return `404 Not Found` with JSON `{"error":"not_found"}`.
- **Bundle size budget**: Production Vite bundle (`web/dist`) stays strictly under 200 KB total gzipped.

## How to get to it (user POV)

Build the frontend (`cd web && npm run build`) and launch `xiaohe core` (with `WEB_DIST_DIR=web/dist` or from the repo root where `./web/dist/index.html` exists), then open `http://127.0.0.1:$PORT/` in a browser or verify routes with `curl`.

## Driving it with `xiaohe-verify.sh` + `curl`

Preconditions: Frontend built via `cd web && npm test && npx tsc --noEmit && npm run build`, then `xiaohe supervisor` + `xiaohe core` launched via `xiaohe-verify.sh launch "$RUN_ID" "$PORT"` (from repo root so `./web/dist` is auto-detected) and `doctor` passed.

```bash
BASE="http://127.0.0.1:${PORT}"

# 1. Verify gzipped bundle size < 200 KB
python3 -c '
import gzip, pathlib
total = sum(len(gzip.compress(p.read_bytes())) for p in pathlib.Path("web/dist").rglob("*") if p.is_file())
print(f"GZIP_TOTAL_BYTES={total}")
assert total < 200_000, f"bundle exceeded 200KB: {total}"
' | tee "$EVIDENCE_DIR/web-bundle-size.txt"

# 2. Root and client-side SPA routes return 200 text/html
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/" | tee "$EVIDENCE_DIR/web-root.txt"
curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE/automations" | tee "$EVIDENCE_DIR/web-spa-route.txt"

# 3. Built JS asset returns 200; missing asset returns 404
JS_ASSET=$(basename "$(ls web/dist/assets/*.js | head -n1)")
curl -sS -o /dev/null -w 'HTTP_STATUS:%{http_code} CONTENT_TYPE:%{content_type}\n' \
  "$BASE/assets/${JS_ASSET}" | tee "$EVIDENCE_DIR/web-asset-ok.txt"
curl -sS -o /dev/null -w 'HTTP_STATUS:%{http_code}\n' \
  "$BASE/assets/nonexistent.js" | tee "$EVIDENCE_DIR/web-asset-404.txt"

# 4. /api, /api/, and /api/does_not_exist return JSON 404 (never index.html)
for p in /api /api/ /api/does_not_exist; do
  curl -sS -w '\nHTTP_STATUS:%{http_code}\n' "$BASE${p}"
done | tee "$EVIDENCE_DIR/web-api-404.txt"
```

## Gotchas

- `web/dist` is git-ignored; always run `cd web && npm run build` (or `make web-build`) before expecting `xiaohe core` to serve the SPA locally.
- Interactive DOM rendering of the four SolidJS views (Dashboard, uPlot History, Skill Manager, Automations) requires a browser; headless CI proves TypeScript compilation (`tsc --noEmit`), snapshot-merge unit tests (`npm test`), and Vite production bundling (`npm run build`), while `curl` proves live Rust SPA serving and `/api` / `/assets` isolation.
