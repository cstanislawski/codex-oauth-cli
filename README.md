# codex-oauth-cli

Minimal standalone Rust CLI for:

- browser OAuth against the same ChatGPT/Codex auth flow used by Codex
- direct requests to `https://chatgpt.com/backend-api/codex/responses`
- reusable prompt templates

Commands:

```bash
cargo run -- login
cargo run -- status
cargo run -- templates init
cargo run -- --template default "Reply with exactly: OK"
```

Template behavior:

- built-ins: `default`, `code-review`, `commit-message`
- custom templates live in `~/.config/codex-oauth-cli/templates`
- `{{prompt}}` placeholder supported
- optional section markers:

```text
--- system ---
You are terse.
--- user ---
{{prompt}}
```

Implementation notes:

- OAuth flow derived from the `openclaw` stack and `@mariozechner/pi-ai` browser flow
- authorize endpoint: `https://auth.openai.com/oauth/authorize`
- token endpoint: `https://auth.openai.com/oauth/token`
- callback: `http://localhost:1455/auth/callback`
- backend endpoint: `https://chatgpt.com/backend-api/codex/responses`
