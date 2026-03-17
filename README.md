# codex-oauth-cli

Minimal standalone Rust CLI for:

- browser OAuth against ChatGPT/Codex
- direct prompts to `https://chatgpt.com/backend-api/codex/responses`

Commands:

```bash
cargo run -- login
cargo run -- status
cargo run -- --model gpt-5.4 "Reply with exactly: OK"
```

Implementation notes:

- OAuth flow derived from the `openclaw` stack and `@mariozechner/pi-ai` browser flow
- authorize endpoint: `https://auth.openai.com/oauth/authorize`
- token endpoint: `https://auth.openai.com/oauth/token`
- callback: `http://localhost:1455/auth/callback`
- backend endpoint: `https://chatgpt.com/backend-api/codex/responses`
