# Naming: pica — November 16, 2026

The product is now **pica**, with **`pica`** as the command. The owner asked for a name "in line with nano and micro" that feels at home beside them, replacing the coined **Eymi** (September 2026), which proved forgettable.

A pica is a printing unit, a sixth of an inch, and the word sounds like pico, the editor nano grew out of. It belongs to the same family of small measures as nano and micro, and it is about type and writing. Four letters, no repeated letter, comfortable to type.

Two Claude Opus instances and Codex first debated the name and settled on `milli` (pico → nano → micro → milli). The owner preferred `pica` and `twip`, because `milli` called a millipede to mind. `milli` was never released.

| Candidate | Rationale | Outcome |
| --- | --- | --- |
| **`pica` — chosen** | A small printing measure that sounds like pico; writing and the size family at once | No Homebrew formula or cask; no command on the checked hosts. Namesakes are a small, stale npm CLI from Pica, an integrations company (`picahq/cli`), and the popular `pica` JavaScript image-resize library; the crates.io name is taken. No terminal editor uses it. Search engines may autocorrect it to pico. |
| `twip` | A twentieth of a point: an even smaller printing unit | The owner's other favorite; more obscure. |
| `milli` | pico → nano → micro → milli | Recalled a millipede. |
| `ronto`, `zepto` | Rare SI prefixes | Too obscure; the same problem as Eymi. |
| `nib`, `mote`, `mica` | Small and writerly | Weaker family tie. |

Taken in the family: `nano`, `micro`, `pico` (Pine), `femto`, `atto`, `kilo` (antirez), `iota`, and `jot` (BSD `/usr/bin/jot`; also a Homebrew formula). `quill`, `tiny`, and `nub` are Homebrew formulae.

The check covered the official [Homebrew formula catalog](https://formulae.brew.sh/api/formula.json) and [cask catalog](https://formulae.brew.sh/api/cask.json), crates.io and npm exact names, executable PATH lookups, and web/GitHub searches. It is a practical collision screen, not a trademark or legal clearance.

## No migration from Eymi

The owner is the only user, so nothing migrates. pica keeps its preferences and recovery snapshots under `pica` directories and ignores any `eymi` or `marklane` directories, leaving them untouched. The Homebrew tap gains `Formula/pica.rb` and drops `Formula/eymi.rb`; an existing installation is replaced with `brew uninstall eymi && brew install yungibly/tap/pica`. GitHub redirects the old `yungibly/eymi` repository URLs.
