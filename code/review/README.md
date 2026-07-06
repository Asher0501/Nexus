# Code Review Directory

This directory contains code-level review artifacts for the Nexus project.

> **Note**: This is distinct from `docs/review/` which houses architecture-level design reviews.
> Here we track **code implementation review findings** — actual code diffs, runtime behavior,
> and discrepancies between the working code and documented design.

## Files

| File | Severity | Description |
|------|----------|-------------|
| `01-critical.md` | 🔴 | Critical issues (must fix before production) |
| `02-moderate.md` | 🟡 | Moderate issues (should fix) |
| `03-low.md` | 🟢 | Low-priority issues (nice to fix) |
| `04-optimizations.md` | 🟢 | Non-urgent optimization opportunities |
| `05-doc-code-drift.md` | 🟡 | Documentation-to-code synchronization gaps |
| `06-doc-fixes-needed.md` | 🟢 | Documentation corrections for outdated review findings |
| `07-deep-review.md` | 🔬 | Deep review — second round, runtime verification |
| `08-third-round.md` | 🔬 | Deep review — third round, fix verification + diagnostics audit |
