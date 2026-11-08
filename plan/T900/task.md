# T900. Update the vulnerable TLS dependency

The creator authorized dependency updates across the audit. Update only vulnerable dependencies and the resolver-required graph without weakening existing guards or advisory policy.

Execution plan: reproduce the advisory audit; apply compatible package-manager updates; review the lockfile diff; run advisory regression checks and the project test/lint/format gates; publish a PR and merge only after required checks pass. No custom dependency implementation is needed. Rollback is a revert of the dependency-update commit.
