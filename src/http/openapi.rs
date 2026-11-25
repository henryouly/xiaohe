use axum::Json;
use axum::response::Html;
use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItemBuilder};
use utoipa::openapi::response::ResponseBuilder;
use utoipa::openapi::{InfoBuilder, OpenApi, OpenApiBuilder, PathsBuilder};

/// Build the OpenAPI 3.1 document describing Xiaohe Core's REST & SSE endpoints.
pub fn build_openapi() -> OpenApi {
    let ok_resp = |desc: &str| ResponseBuilder::new().description(desc).build();

    let mut paths = PathsBuilder::new();
    let specs: &[(&str, HttpMethod, &str, &str)] = &[
        ("/health", HttpMethod::Get, "Liveness probe", "Health"),
        (
            "/health/ready",
            HttpMethod::Get,
            "Readiness probe (storage + runtime)",
            "Health",
        ),
        (
            "/api/auth/bootstrap",
            HttpMethod::Post,
            "First-boot initial Admin creation",
            "Auth",
        ),
        (
            "/api/auth/login",
            HttpMethod::Post,
            "Login with username/password (sets HttpOnly cookie)",
            "Auth",
        ),
        (
            "/api/auth/logout",
            HttpMethod::Post,
            "Revoke session and clear cookie",
            "Auth",
        ),
        (
            "/api/auth/me",
            HttpMethod::Get,
            "Current auth status and user summary",
            "Auth",
        ),
        (
            "/api/areas",
            HttpMethod::Get,
            "List areas allowed for caller",
            "Areas",
        ),
        (
            "/api/areas",
            HttpMethod::Post,
            "Create area (Admin)",
            "Areas",
        ),
        (
            "/api/areas/{area_id}",
            HttpMethod::Put,
            "Update area (Admin)",
            "Areas",
        ),
        (
            "/api/areas/{area_id}",
            HttpMethod::Delete,
            "Delete area (Admin)",
            "Areas",
        ),
        (
            "/api/dashboard",
            HttpMethod::Get,
            "Get Dashboard widget layout",
            "Dashboard",
        ),
        (
            "/api/dashboard",
            HttpMethod::Put,
            "Update Dashboard widget layout (Admin)",
            "Dashboard",
        ),
        (
            "/api/entities",
            HttpMethod::Get,
            "List entities filtered by RBAC allowed_areas",
            "Entities",
        ),
        (
            "/api/entities/{entity_id}",
            HttpMethod::Get,
            "Get single entity (area-scoped)",
            "Entities",
        ),
        (
            "/api/entities/{entity_id}/command",
            HttpMethod::Post,
            "Dispatch command to entity",
            "Entities",
        ),
        (
            "/api/entities/{entity_id}/history",
            HttpMethod::Get,
            "Query numeric or discrete entity history",
            "History",
        ),
        (
            "/api/skills",
            HttpMethod::Get,
            "List discovered skills and status (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}",
            HttpMethod::Get,
            "Get skill manifest, config, and secret status (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/config",
            HttpMethod::Put,
            "Update skill config and write-only secrets (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/start",
            HttpMethod::Post,
            "Start device skill (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/stop",
            HttpMethod::Post,
            "Stop device skill (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/restart",
            HttpMethod::Post,
            "Restart device skill (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/reinstall",
            HttpMethod::Post,
            "Reinstall skill uv environment (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/logs",
            HttpMethod::Get,
            "Fetch skill stderr ring-buffer logs (Admin)",
            "Skills",
        ),
        (
            "/api/skills/{skill_id}/invoke",
            HttpMethod::Post,
            "Invoke action skill (Admin)",
            "Skills",
        ),
        (
            "/api/automations",
            HttpMethod::Get,
            "List declarative TOML automations (Admin)",
            "Automations",
        ),
        (
            "/api/automations",
            HttpMethod::Post,
            "Create automation from raw TOML (Admin)",
            "Automations",
        ),
        (
            "/api/automations/runs",
            HttpMethod::Get,
            "Query automation execution audit runs (Admin)",
            "Automations",
        ),
        (
            "/api/automations/reload",
            HttpMethod::Post,
            "Hot-reload automations from disk (Admin)",
            "Automations",
        ),
        (
            "/api/automations/{id}",
            HttpMethod::Get,
            "Get parsed and raw TOML automation (Admin)",
            "Automations",
        ),
        (
            "/api/automations/{id}",
            HttpMethod::Put,
            "Update automation from raw TOML (Admin)",
            "Automations",
        ),
        (
            "/api/automations/{id}",
            HttpMethod::Delete,
            "Delete automation (Admin)",
            "Automations",
        ),
        (
            "/api/automations/{id}/toggle",
            HttpMethod::Post,
            "Enable or disable automation (Admin)",
            "Automations",
        ),
        (
            "/api/automations/{id}/trigger",
            HttpMethod::Post,
            "Manually trigger automation (Admin)",
            "Automations",
        ),
        (
            "/api/users",
            HttpMethod::Get,
            "List household users (Admin)",
            "Users",
        ),
        (
            "/api/users",
            HttpMethod::Post,
            "Create user (Admin)",
            "Users",
        ),
        (
            "/api/users/{user_id}",
            HttpMethod::Put,
            "Update user role/areas/password (Admin)",
            "Users",
        ),
        (
            "/api/users/{user_id}",
            HttpMethod::Delete,
            "Delete user (Admin)",
            "Users",
        ),
        (
            "/api/users/{user_id}/tokens",
            HttpMethod::Post,
            "Mint API Bearer token (Admin)",
            "Users",
        ),
        (
            "/api/users/{user_id}/tokens/{token_id}",
            HttpMethod::Delete,
            "Revoke API Bearer token (Admin)",
            "Users",
        ),
        (
            "/api/webhooks/{webhook_id}",
            HttpMethod::Post,
            "Ingest authenticated HTTP webhook",
            "Webhooks",
        ),
        (
            "/api/events",
            HttpMethod::Get,
            "RBAC-filtered Server-Sent Events stream",
            "Events",
        ),
    ];

    for (path, method, summary, tag) in specs {
        let op = OperationBuilder::new()
            .summary(Some(*summary))
            .tag(*tag)
            .response("200", ok_resp("Success"))
            .build();
        paths = paths.path(
            *path,
            PathItemBuilder::new().operation(method.clone(), op).build(),
        );
    }

    OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .title("Xiaohe Core API")
                .version(env!("CARGO_PKG_VERSION"))
                .description(Some(
                    "Self-hosted smart home control platform REST & SSE API (OpenAPI 3.1)",
                ))
                .build(),
        )
        .paths(paths.build())
        .build()
}

/// `GET /api/openapi.json` — OpenAPI 3.1 JSON schema.
pub async fn openapi_json() -> Json<OpenApi> {
    Json(build_openapi())
}

/// `GET /docs` — lightweight interactive HTML API documentation page.
pub async fn docs_html() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width,initial-scale=1" />
  <title>Xiaohe Core API Docs</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 2rem auto; max-width: 960px; padding: 0 1rem; color: #111827; }
    h1 { margin-bottom: 0.25rem; }
    .meta { color: #4b5563; margin-bottom: 1.5rem; }
    .ep { border: 1px solid #e5e7eb; border-radius: 8px; padding: 0.75rem 1rem; margin-bottom: 0.5rem; display: flex; gap: 0.75rem; align-items: baseline; }
    .m { font-weight: 700; font-size: 0.8rem; padding: 0.15rem 0.5rem; border-radius: 4px; background: #eef2ff; color: #3730a3; text-transform: uppercase; min-width: 64px; text-align: center; }
    code { font-family: ui-monospace, monospace; font-size: 0.9rem; }
    .sum { color: #374151; margin-left: auto; font-size: 0.9rem; }
  </style>
</head>
<body>
  <h1>Xiaohe Core API</h1>
  <p class="meta">OpenAPI 3.1 specification: <a href="/api/openapi.json"><code>/api/openapi.json</code></a></p>
  <div id="list">Loading OpenAPI 3.1 spec…</div>
  <script>
    fetch('/api/openapi.json').then(r => r.json()).then(spec => {
      const el = document.getElementById('list');
      el.innerHTML = '';
      for (const [path, item] of Object.entries(spec.paths || {})) {
        for (const [method, op] of Object.entries(item)) {
          const row = document.createElement('div');
          row.className = 'ep';
          row.innerHTML = `<span class="m">${method}</span><code>${path}</code><span class="sum">${op.summary || ''}</span>`;
          el.appendChild(row);
        }
      }
    });
  </script>
</body>
</html>"#,
    )
}
