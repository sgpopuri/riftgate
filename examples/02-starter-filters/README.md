# 02-starter-filters

Starter catalog for the v0.3 filter-workflow surface.

This example is intentionally lightweight: it documents the baseline filter
chain shape and starter filters while the production `WasmFilter` runtime is
still landing.

## What this example covers

- Filter chain order contract (`request`: first -> last, `response`: last -> first).
- Starter filter intents:
  - `pii_redactor`
  - `prompt_template_substitution`
  - `output_schema_validator`
  - `cost_guard`
  - `token_budget_guard`
- Config shape operators can stage now, even before WASM backend wiring is complete.

## Status

- `FilterChain` executor: shipped.
- `WasmFilter` backend: scaffold (load path intentionally returns `BackendNotWired`).

## Sample config shape

```toml
[filters]
enabled = true

[[filters.chain]]
name = "pii_redactor"
kind = "wasm"
component = "./filters/pii-redactor.wasm"
mode = "request"

[[filters.chain]]
name = "schema_validator"
kind = "wasm"
component = "./filters/schema-validator.wasm"
mode = "response"
```

## Running notes

Use this example as a documentation and planning surface until the production
`WasmFilter` host runtime is landed. The chain order semantics and filter
naming contract are stable; only runtime execution of WASM components is
pending.
