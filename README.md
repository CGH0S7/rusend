# rusend

A small, user-friendly command-line client for the Resend API (Rust).

## Features
- Save API key (`rusend config --key re_xxx`)
- Send single email, read body from stdin or `--html`/`--text`
- Send batch from JSON file
- List, get, update, cancel sent emails
- List and get received emails (inbox)

## Build

```bash
cargo build --release
```

## Examples

Save API key:

```bash
rusend config --key re_xxxxxxxxx
```

Send an email (body from stdin):

```bash
echo "<p>hello</p>" | rusend send -f "Acme <no-reply@acme.com>" -t "you@example.com" -s "hi" --from-stdin
```

Send a batch: create `batch.json` with an array of objects like:

```json
[
  {"from":"Acme <onboarding@resend.dev>", "to":["a@example.com"], "subject":"hello", "html":"<p>hi</p>"}
]
```

Then:

```bash
rusend batch batch.json
```
