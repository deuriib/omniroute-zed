# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |

## Reporting a Vulnerability

Open a GitHub Security Advisory on this repository, or email
<deuriib@gmail.com> with `[omniroute-zed] security` in the subject.

What to include:

- Affected version / commit
- Steps to reproduce (no live secrets — redact API keys)
- Impact assessment, if known

What to expect:

- Acknowledgement within 72 hours
- Fix or mitigation plan with an agreed timeline
- Coordinated disclosure: no public details until a fix ships

## Handling Notes

- Never put real API keys, tokens, or `settings.json` contents in issues,
  PRs, logs, or test fixtures. The syncer reads `OMNIROUTE_API_KEY` from
  the environment and never requires it in config files.
- Zed reads `OMNIROUTE_API_KEY` on its own for the `omniroute` provider;
  prefer the environment or OS keychain over `--write-api-key`.
