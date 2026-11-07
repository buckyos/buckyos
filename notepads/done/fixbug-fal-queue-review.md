# Fal queue review fixes

## Root causes

- Fal file inputs were declared URL-only even though the official API accepts Base64 Data URI values, which rejected inline and locally materialized resources.
- Lifecycle URL construction truncated `workflows/<owner>/<name>` and `comfy/<owner>/<name>` endpoints to two path segments.
- The acceptance contract omitted the `/response` result URL returned by queue submission and contained stale endpoint-specific lifecycle fixtures.

## Fix

- Encode Base64 and byte-backed named objects as MIME-preserving Data URI values while preferring an available HTTP(S) materialized URL.
- Preserve three endpoint segments for the `workflows` and `comfy` namespaces and retain two segments for normal Fal model endpoints.
- Validate both the canonical result endpoint and the returned `/response` alias, correct lifecycle fixtures, and make the Mock tests follow the URLs returned by submit.

## Regression coverage

- Fal codec unit tests cover URL preference, Base64 and named-object fallback, invalid Base64, namespaced endpoints, and lifecycle URL construction.
- Acceptance self-tests cover all four Fal contracts, Data URI validation, returned lifecycle links, canonical result aliases, and stale endpoint rejection.
