<!--
Thanks for opening a pull request! Filling this in helps reviewers
understand the change in context and catches the small things that
would otherwise come back as review comments.
-->

## Summary

<!-- One paragraph: what does this change do, and why? -->

## Linked issue

<!-- "Closes #123" or "Refs #123". Delete this section if not applicable. -->

## Test plan

<!--
What did you test, and how? Reviewers should be able to reproduce it.
Add new automated tests when the change can be covered by them.
-->

- [ ]
- [ ]
- [ ]

## Quality gates (must be green before merge)

- [ ] `cargo fmt --check --all`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo deny check` (run locally if changing dependencies)
- [ ] `birdnet-behavior --doctor` still exits 0 on a known-good config

## Behavioural impact

- [ ] No user-visible behaviour change
- [ ] Adds a new user-visible behaviour (documented in README / .env.example)
- [ ] Changes existing user-visible behaviour (CHANGELOG updated, migration notes included if needed)
- [ ] Breaking change (CHANGELOG documents the break and the upgrade path)

## Security & supply chain

- [ ] No new runtime dependencies
- [ ] New runtime dependencies: licensed compatibly with the project (see `deny.toml`), justified above
- [ ] Touches network I/O / auth / file paths / subprocess invocation — added input validation and a test

## Documentation

- [ ] No documentation needed
- [ ] Updated `README.md`
- [ ] Updated `.env.example`
- [ ] Added or updated an Architecture Decision Record under `docs/architecture/`
- [ ] Updated `CHANGELOG.md`
