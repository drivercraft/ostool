pub const ADMIN_APP_JS: &str = r#"
window.fetchJson = async function fetchJson(url, options) {
  const response = await fetch(url, options);
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(body.message || `Request failed: ${response.status}`);
  }
  return body;
};

window.renderJson = function renderJson(id, value) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = JSON.stringify(value, null, 2);
  }
};
"#;

pub fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <style>
      :root {{
        --bg: #f6f3ee;
        --panel: #fffaf2;
        --ink: #1d1a17;
        --muted: #6d6258;
        --accent: #0d6b56;
        --accent-soft: #d8efe7;
        --line: #d8cfc3;
      }}
      body {{
        margin: 0;
        font-family: Georgia, "Times New Roman", serif;
        color: var(--ink);
        background:
          radial-gradient(circle at top left, #fbe8cf 0, transparent 35%),
          linear-gradient(180deg, #f4efe6 0%, #ece6db 100%);
      }}
      header {{
        padding: 24px 32px;
        border-bottom: 1px solid var(--line);
        background: rgba(255, 250, 242, 0.88);
        backdrop-filter: blur(8px);
      }}
      nav a {{
        margin-right: 16px;
        color: var(--accent);
        text-decoration: none;
        font-weight: 600;
      }}
      main {{
        padding: 24px 32px 40px;
      }}
      .panel {{
        background: var(--panel);
        border: 1px solid var(--line);
        border-radius: 16px;
        padding: 20px;
        box-shadow: 0 14px 32px rgba(68, 51, 31, 0.08);
      }}
      pre {{
        padding: 16px;
        border-radius: 12px;
        background: #f2ece2;
        overflow: auto;
      }}
      h1, h2 {{
        margin-top: 0;
      }}
      .muted {{
        color: var(--muted);
      }}
    </style>
  </head>
  <body>
    <header>
      <h1>{title}</h1>
      <nav>
        <a href="/admin/boards">Boards</a>
        <a href="/admin/sessions">Sessions</a>
        <a href="/admin/tftp">TFTP</a>
      </nav>
    </header>
    <script>{}</script>
    <main>{body}</main>
  </body>
</html>"#,
        ADMIN_APP_JS
    )
}
