# shiioo-secrets

The office's secret store: tokens and keys the agents need, encrypted at
rest in `.shiioo/`, referenced by name everywhere else. Design:
[security.md §3.2](../../docs/design/security.md#32-secrets-never-reach-the-record-built-never-reach-the-model-holds-for-the-native-loop-planned),
[ADR-0011](../../docs/adr/0011-containers-through-the-owners-engine.md).

## Position in the crate graph

```
shiioo-secrets  ──►  shiioo (binary)
```

Nothing else depends on it. The runtime defines the `SecretSource` trait it
needs (`shiioo-runtime::secrets`); the binary implements that trait over
`SecretStore`, so the runtime stays free of file formats and crypto.

## Files

| File | Contents | Mode |
| --- | --- | --- |
| `.shiioo/secrets.key` | 32 random bytes, hex, made on first use | 0600 |
| `.shiioo/secrets.enc` | 24-byte random nonce, then the name → value JSON map sealed with XChaCha20-Poly1305 under the key | 0600 |

Both are written through a temporary file and a rename. Losing the key
file makes the data unreadable (`SecretError::Corrupt`); back them up
together, or not at all and set the secrets again.

## API

```rust
let store = SecretStore::open(&data_dir)?;        // creates the key on first use
store.set("gh-token", "ghp_…")?;
store.get("gh-token")?;                           // Option<String>
store.remove("gh-token")?;                        // bool
store.names()?;                                   // Vec<String>, sorted
store.all()?;                                     // Vec<(name, value)>, for redaction
shiioo_secrets::validate("gh-token")?;            // [a-z0-9][a-z0-9._-]{0,63}
```

Names are lower-case letters, digits, `.`, `_` and `-`, so they are safe in
`secret:<name>` references, file names and shell lines.

## CLI

In the binary: `shiioo secret set <name> [--from-env VAR | --from-file
PATH]` (default: stdin), `shiioo secret list`, `shiioo secret rm <name>`.
Values never come from the command line, where they would end up in shell
history and process listings.

## Testing

```sh
cargo test -p shiioo-secrets
```

Round trip across opens, file modes, the value not being in the file in the
clear, a different key failing to decrypt, name validation.
