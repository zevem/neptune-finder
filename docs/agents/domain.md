# Domain docs

Neptune Finder uses a single domain context across the desktop application, `finder-core`, and `finder-operations`.

## Layout and reading

- `GLOSSARY.md` at the repository root holds domain vocabulary. Read it, when present, before work that names or changes domain concepts.
- `docs/adr/` holds architectural decision records. Read accepted ADRs relevant to the area being changed.
- Follow the root [AGENTS.md](../../AGENTS.md) contextual-reading table for existing guidance. In particular, [docs/architecture.md](../architecture.md) remains the source of truth for ownership, dependencies, engine, and renderer decisions. Read the owning directory's guide for the boundary being changed.

When the glossary or ADR directory is absent, proceed silently. The domain-modeling skill creates them lazily as terms and decisions are resolved; setup creates no empty glossary or ADR files.

## Use the glossary's vocabulary

Use glossary terms in issue titles, refactor proposals, hypotheses, and test names. If a needed concept is missing, check the project's existing language before recording a real gap for domain-modeling.

## Surface decision conflicts

Surface conflicts with approved product or architectural decisions and relevant accepted ADRs. Reopening a decision requires an explicit decision; existing implementation and dated review reports do not replace that authority.
