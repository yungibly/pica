# Naming: milli — November 16, 2026

The product is now **milli**, with **`milli`** as the command. The owner asked for a name "in line with nano and micro" that feels at home beside them, replacing the coined **Eymi** (September 2026), which proved forgettable.

The lineage explains itself: pico → nano → micro → milli, each prefix a thousand times larger. "A little more than micro" is the product: micro's keys and launch behavior, nano's quiet, and Markdown rendered as you write. It also reads as a name ("Milly"). Two Claude Opus instances and Codex debated the choice; all three settled on `milli`.

| Candidate | Rationale | Outcome |
| --- | --- | --- |
| **`milli` — chosen** | Self-explanatory in the nano/micro family; five familiar letters | No Homebrew formula or cask. An archived Meilisearch crate holds the crates.io name, irrelevant to a Homebrew binary. A small hobby C project, `winterrdog/milli_editor`, shares the word. |
| `pica` | A typographic unit that sounds like pico; fits writing | Fallback if the doubled `l` grates. The popular `pica` JS image library shares the name. |
| `ronto` | 2022 SI prefix; types cleanly and ends in -o | Too obscure: it would need the same explanation Eymi did. |
| `nib`, `mote` | Small and writerly | Weak family tie. |

Taken in the family: `nano`, `micro`, `pico` (Pine), `femto`, `atto`, `kilo` (antirez), `iota`, and `jot` (BSD `/usr/bin/jot`; also a Homebrew formula). `quill`, `tiny`, and `nub` are Homebrew formulae.

The check covered the official [Homebrew formula catalog](https://formulae.brew.sh/api/formula.json) and [cask catalog](https://formulae.brew.sh/api/cask.json), crates.io and npm exact names, and web/GitHub searches. It is a practical collision screen, not a trademark or legal clearance.

## Compatibility with earlier names

Preferences and recovery snapshots are looked up under `milli`, then `eymi`, then `marklane`, without migrating or deleting anything; the newest existing directory wins even when empty. The Homebrew tap maps `eymi` to `milli` through `formula_renames.json`, so `brew upgrade` follows the rename. GitHub redirects the old `yungibly/eymi` repository URLs.
