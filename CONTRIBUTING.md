# Contributing

Thanks for stopping by. Small disciplines, big craftsmanship.

## Ground Rules

- Work in English for code, comments, docs, and commits.
- One concern per PR. Keep it small enough to review in one sitting.
- No drive-by reformatting. Touch only the lines your change needs.
- Never commit secrets, API keys, `settings.json`, or `.env` files.

## Workflow

1. Fork, branch off `main` (`feat/<slug>`, `fix/<slug>`, `docs/<slug>`).
2. Run the gates before pushing:
   ```sh
   mise run check   # fmt + clippy -D warnings + tests
   ```
3. Open a PR against `main` with:
   - What changed and why (link the issue when one exists)
   - How you verified it (commands + output, not "works for me")
   - Docs updated (`README.md` / `CHANGELOG.md` when user-facing)

## Conventions

- Commits follow [Conventional Commits](https://www.conventionalcommits.org/):
  `feat:`, `fix:`, `docs:`, `chore:`, `ci:`, `test:`, `refactor:`.
- User-facing changes get a `CHANGELOG.md` entry under `[Unreleased]`.
- Tests ship with behavior changes. Docs-only changes are exempt.
- Coverage floors: line >= 80%, critical paths (settings merge, auth
  handling) higher. Count-only assertions do not count — verify invariants.

## Reporting Issues

Include: version (`omniroute-zed --version`), OS, OmniRoute gateway
version/URL, redacted config (no keys), exact command, and full error
output. "It doesn't work" gets sent back with love and a checklist.
