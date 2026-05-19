# Security Policy

## Supported Versions

kyma is in pre-alpha. Only the latest commit on `main` is supported; older tags
and branches are not maintained. Production use is not yet recommended.

## Reporting a Vulnerability

Please report security issues **privately**, not via public GitHub issues.

- Open a private security advisory:
  <https://github.com/shakedaskayo/kyma/security/advisories/new>
- Or email **shaked@agentcylabs.com** with the subject prefix `[kyma security]`.

Please include:

- A description of the issue and its impact.
- Steps to reproduce (a minimal proof-of-concept is ideal).
- The commit SHA you tested against.
- Any suggested mitigation, if you have one.

We will acknowledge new reports within **3 business days** and aim to provide
an initial assessment within **10 business days**. Coordinated disclosure
timelines are agreed case-by-case; we appreciate a reasonable window before
public disclosure.

## Scope

In scope:

- The kyma engine binary and all crates under `crates/`.
- The web UI under `web/`.
- The default Docker image and `docker-compose.yml` development stack.
- Documented HTTP, Arrow Flight, and OTLP endpoints.

Out of scope:

- Third-party dependencies (please report upstream).
- Issues that require pre-existing root or local network access to the host.
- Denial of service from intentionally pathological queries; query budgets are
  the supported mitigation.
- Social engineering of contributors or infrastructure providers.
