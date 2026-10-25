# shiioo-mail

Outgoing mail for the office: SMTP with the owner's account through
`lettre`, or a directory of `.eml` files for tests and dry runs. Who may
send what is decided in `shiioo-api::mail` (the tool `mail.send`, judged by
the rules, approved by the owner); this crate only delivers.

## API

- `SmtpConfig { host, port, security: Security::{StartTls, Tls, None}, from,
  user, password }`, `Mailer::smtp(&config)`.
- `Mailer::file(dir, from)`: writes `<message id>.eml` instead of sending;
  `sent_files(dir)` lists them.
- `Mailer::send(&Mail { to, cc, subject, body }) -> message id`. The body is
  plain text, sent as `multipart/alternative` with a minimal HTML copy.
- `Security::parse("starttls" | "tls" | "none")`.

## Testing

```sh
cargo test -p shiioo-mail
```

Writes a message through the file transport and checks every header and
both bodies are in it; bad and missing addresses are errors.
